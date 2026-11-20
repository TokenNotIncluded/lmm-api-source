"""Archive tests use synthetic ELF fixtures, not claims of real binary builds."""
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import shutil
import struct
import tarfile
import tempfile
import tomllib
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("distribution", ROOT / "scripts/build-distribution.py")
DIST = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(DIST)
SHA = "a" * 40


def elf(machine=62):
    data = bytearray(64)
    data[:6] = b"\x7fELF\x02\x01"
    struct.pack_into("<H", data, 18, machine)
    return bytes(data)


class DistributionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.inputs = self.root / "inputs"
        self.inputs.mkdir()
        for name in ("lmm-core", "lmm-extensions", "lmm"):
            (self.inputs / name).write_bytes(elf())
        (self.inputs / "index.html").write_text("<!doctype html><title>Test fixture</title>")

    def build(self, component="extensions", **changes):
        args = dict(root=ROOT, component=component, version="0.1.0-preview", revision=SHA,
                    platform="any" if component == "web" else "linux-amd64",
                    inputs=self.inputs, output=self.root / "output", epoch=0)
        args.update(changes)
        return DIST.build(**args)

    def test_each_component_has_exact_binaries_and_licenses(self):
        expected_binaries = {"core": {"bin/lmm-core"}, "extensions": {"bin/lmm-extensions"},
                             "cli": {"bin/lmm"}, "web": set()}
        for component, expected in expected_binaries.items():
            with self.subTest(component=component):
                archive = self.build(component)
                manifest = DIST.verify(archive, SHA)
                files = manifest["files"]
                self.assertEqual({name for name in files if name.startswith("bin/")}, expected)
                self.assertEqual({name for name, info in files.items() if info["mode"] == 0o755}, expected)
                for name in ("LICENSE", "NOTICE", "THIRD-PARTY-LICENSES.md"):
                    self.assertIn(name, files)
                self.assertNotIn("lmm-api-web.install", files)
                checksum = archive.with_name(archive.name + ".sha256").read_text()
                self.assertEqual(checksum.split()[0], hashlib.sha256(archive.read_bytes()).hexdigest())

    def test_reproducible_archive_and_contents(self):
        first = self.build(output=self.root / "first")
        second = self.build(output=self.root / "second")
        self.assertEqual(first.read_bytes(), second.read_bytes())
        with tarfile.open(first) as archive:
            config = tomllib.loads(archive.extractfile("config/legal/site.example.toml").read().decode("utf-8"))
            self.assertFalse(config["published"])
            self.assertTrue(all(value == "" for value in config["variables"].values()))
        self.assertNotIn("config/legal/legal.toml", DIST.verify(first)["files"])
        self.assertNotIn("config/legal/site.toml", DIST.verify(first)["files"])

    def test_cli_archive_preserves_port_attribution_and_magpie_mit_license(self):
        archive = self.build("cli")
        manifest = DIST.verify(archive, SHA)
        names = ("apps/cli/NOTICE", "apps/cli/third_party/magpie/LICENSE",
                 "apps/cli/third_party/magpie/NOTICE.md")
        with tarfile.open(archive) as tar:
            for name in names:
                with self.subTest(name=name):
                    self.assertEqual(tar.extractfile(name).read(), (ROOT / name).read_bytes())
                    self.assertEqual(manifest["files"][name]["mode"], 0o644)
            license_text = tar.extractfile(names[1]).read()
            self.assertIn(b"Copyright (c) 2026 yetone", license_text)
            self.assertIn(b'THE SOFTWARE IS PROVIDED "AS IS"', license_text)

    def test_no_overwrite(self):
        first = self.build()
        original = first.read_bytes()
        with self.assertRaises(FileExistsError):
            self.build()
        self.assertEqual(first.read_bytes(), original)

    def test_missing_binary_and_wrong_architecture(self):
        (self.inputs / "lmm-core").unlink()
        with self.assertRaises(OSError):
            self.build("core")
        (self.inputs / "lmm-extensions").write_bytes(elf(183))
        with self.assertRaises(ValueError):
            self.build()

    def test_symlink_input_rejected(self):
        (self.inputs / "lmm-extensions").unlink()
        (self.inputs / "lmm-extensions").symlink_to(self.inputs / "lmm")
        with self.assertRaises(ValueError):
            self.build()

    def test_web_rejects_secrets_and_links(self):
        for name in (".env", "private.key", "site.json", "site.toml", "legal.toml", "symlink"):
            with self.subTest(name=name):
                path = self.inputs / name
                if name == "symlink":
                    path.symlink_to(self.inputs / "index.html")
                else:
                    path.write_text("not for distribution")
                with self.assertRaises(ValueError):
                    self.build("web")
                path.unlink()

    def test_public_discovery_is_preserved_without_allowing_hidden_files(self):
        public = self.inputs / ".well-known"
        public.mkdir()
        source = ROOT / "apps/web/public/.well-known/webmcp.json"
        shutil.copyfile(source, public / "webmcp.json")
        archive = self.build("web", output=self.root / "discovery")
        manifest = DIST.verify(archive, SHA)
        self.assertIn("dist/.well-known/webmcp.json", manifest["files"])
        for name in (".well-known/.env", ".well-known/private.json",
                     "dist/.well-known/.env", "dist/.well-known/private.json",
                     "config/.well-known/webmcp.json"):
            with self.subTest(name=name), self.assertRaises(ValueError):
                DIST.safe_path(name)
        (public / "private.json").write_text("not public")
        with self.assertRaises(ValueError):
            self.build("web", output=self.root / "private")

    def test_identity_validation(self):
        for changes in ({"version":"../../x"}, {"revision":"short"}, {"platform":"any"}, {"epoch":-1}):
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                self.build(**changes)
        with self.assertRaises(ValueError):
            DIST.verify(self.build(), "b" * 40)

    def test_unsafe_paths(self):
        for name in ("/abs", "../escape", "a/../escape", "a//b", "a\\b", ".env", "a/.secrets/key", "a\nb", "node_modules/x"):
            with self.subTest(name=name), self.assertRaises(ValueError):
                DIST.safe_path(name)

    def test_tamper_extra_files_and_links_rejected(self):
        original = self.build()
        for change in ("tamper", "extra", "link", "duplicate", "traversal"):
            path = self.root / (change + ".tar.gz")
            with tarfile.open(original) as source, tarfile.open(path, "w:gz") as target:
                for entry in source:
                    data = source.extractfile(entry).read()
                    if change == "tamper" and entry.name == "README.md":
                        data = b"x" * len(data)
                    target.addfile(entry, io.BytesIO(data))
                if change in {"extra", "duplicate", "traversal"}:
                    entry = tarfile.TarInfo({"extra":"extra", "duplicate":"README.md", "traversal":"../escape"}[change])
                    entry.mode, entry.size = 0o644, 1
                    target.addfile(entry, io.BytesIO(b"x"))
                elif change == "link":
                    entry = tarfile.TarInfo("linked")
                    entry.type, entry.linkname = tarfile.SYMTYPE, "/etc/passwd"
                    target.addfile(entry)
            with self.subTest(change=change), self.assertRaises(ValueError):
                DIST.verify(path)

    def test_only_three_product_executables_are_declared(self):
        spec = json.loads((ROOT / "packaging/distribution.json").read_text())
        self.assertEqual(set(spec["components"]), {"core", "extensions", "cli", "web"})
        self.assertEqual([name for component in spec["components"].values()
                          for name in component["binaries"]], ["lmm-core", "lmm-extensions", "lmm"])

    def test_manifest_cannot_authorize_an_extra_executable(self):
        original = self.build("core")
        for name in ("bin/lmm-core-admin", "bin/lmm-api-deploy", "bin/lmm-api-go-deploy", "tools/deploy"):
            path = self.root / (name.replace("/", "-") + ".tar.gz")
            data = elf()
            with tarfile.open(original) as source, tarfile.open(path, "w:gz") as target:
                manifest = json.load(source.extractfile("MANIFEST.json"))
                manifest["files"][name] = {"sha256": hashlib.sha256(data).hexdigest(),
                                           "size": len(data), "mode": 0o755}
                for entry in source:
                    payload = source.extractfile(entry).read()
                    if entry.name == "MANIFEST.json":
                        payload = (json.dumps(manifest) + "\n").encode()
                        entry.size = len(payload)
                    target.addfile(entry, io.BytesIO(payload))
                entry = tarfile.TarInfo(name)
                entry.mode, entry.size = 0o755, len(data)
                target.addfile(entry, io.BytesIO(data))
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, "executable"):
                DIST.verify(path)

    def test_resource_manifest_only_contains_existing_reviewed_files(self):
        spec = json.loads((ROOT / "packaging/distribution.json").read_text())
        for component in spec["components"].values():
            for name in component["resources"] + spec["common"]:
                self.assertTrue((ROOT / name).is_file(), name)
                self.assertFalse((ROOT / name).is_symlink(), name)
        resources = spec["components"]["extensions"]["resources"]
        self.assertEqual(sum(name.startswith("config/legal/templates/") for name in resources), 6)


    def extore_fixture(self, arch="amd64"):
        """Synthetic headers and readelf text test packaging, never native execution."""
        machine = {"amd64": 62, "arm64": 183}[arch]
        for name, kind in (("lmm-extensions", 2), ("lmm-extore-host", 2), ("libextore_zplugin.so", 3)):
            data = bytearray(elf(machine))
            struct.pack_into("<H", data, 16, kind)
            (self.inputs / name).write_bytes(data)
        source = self.root / "repo"
        spec = json.loads((ROOT / "packaging/distribution.json").read_text())
        names = spec["common"] + spec["components"]["extensions"]["resources"]
        names += spec["components"]["extensions"]["optional_extore_plugin"]["resources"]
        for name in names:
            path = source / name
            path.parent.mkdir(parents=True, exist_ok=True)
            if name == "third_party/extore/LICENSE":
                path.write_text("Synthetic license fixture, not a distributed product license.\n")
            else:
                shutil.copyfile(ROOT / name, path)
        shutil.copytree(ROOT / "packaging", source / "packaging")
        record = DIST.record_extore(self.inputs, SHA, "e" * 40,
                                   spec["components"]["extensions"]["optional_extore_plugin"]["targets"]["linux-" + arch])
        (self.inputs / "extore-plugin.json").write_text(json.dumps(record))
        return source, record

    def test_optional_extore_pair_has_exact_provenance_modes_and_distinct_name(self):
        source, record = self.extore_fixture()
        archive = self.build(root=source, extore_revision="e" * 40)
        self.assertTrue(archive.name.endswith("-linux-amd64-extore.tar.gz"))
        manifest = DIST.verify(archive, SHA)
        self.assertEqual(manifest["extore_plugin"], record)
        self.assertEqual(manifest["files"]["libexec/lmm-extore-host"]["mode"], 0o755)
        self.assertEqual(manifest["files"]["lib/extore/libextore_zplugin.so"]["mode"], 0o644)
        self.assertEqual({p for p in manifest["files"] if p.startswith("bin/")}, {"bin/lmm-extensions"})
        self.assertIn("third_party/extore/LICENSE", manifest["files"])
        self.assertIn("docs/extore.md", manifest["files"])
        with self.assertRaisesRegex(ValueError, "explicit"):
            self.build(root=source, output=self.root / "implicit")

    def test_optional_pair_requires_both_files_and_matching_immutable_inputs(self):
        source, record = self.extore_fixture()
        record_path = self.inputs / "extore-plugin.json"
        for field, value in (("source_revision", "b" * 40), ("extore_revision", "c" * 40),
                             ("target", "aarch64-unknown-linux-gnu"), ("schema_version", True)):
            with self.subTest(field=field), self.assertRaises(ValueError):
                changed = dict(record)
                changed[field] = value
                record_path.write_text(json.dumps(changed))
                self.build(root=source, extore_revision="e" * 40)
        record_path.write_text(json.dumps(record))
        for filename in ("lmm-extore-host", "libextore_zplugin.so", "extore-plugin.json"):
            path = self.inputs / filename
            original = path.read_bytes()
            path.unlink()
            with self.subTest(file=filename), self.assertRaises(OSError):
                self.build(root=source, extore_revision="e" * 40)
            path.write_bytes(original)
        library = self.inputs / "libextore_zplugin.so"
        library.write_bytes(library.read_bytes() + b"changed")
        with self.assertRaisesRegex(ValueError, "differs"):
            self.build(root=source, extore_revision="e" * 40)

    def test_optional_pair_rejects_nonshared_library_and_other_components(self):
        source, _ = self.extore_fixture()
        with self.assertRaisesRegex(ValueError, "extensions-only"):
            self.build("core", root=source, extore_revision="e" * 40)
        data = bytearray((self.inputs / "libextore_zplugin.so").read_bytes())
        struct.pack_into("<H", data, 16, 2)
        (self.inputs / "libextore_zplugin.so").write_bytes(data)
        with self.assertRaisesRegex(ValueError, "shared library"):
            self.build(root=source, extore_revision="e" * 40)

    def test_optional_arm64_target_is_bound_and_wrong_architecture_fails(self):
        source, record = self.extore_fixture("arm64")
        self.assertEqual(record["target"], "aarch64-unknown-linux-gnu")
        archive = self.build(root=source, platform="linux-arm64", extore_revision="e" * 40)
        DIST.verify(archive, SHA)
        with self.assertRaises(ValueError):
            self.build(root=source, platform="linux-amd64", extore_revision="e" * 40)

    def test_native_conversion_installs_private_pair_with_measured_dependencies(self):
        source, record = self.extore_fixture()
        archive = self.build(root=source, extore_revision="e" * 40)
        spec = importlib.util.spec_from_file_location("native_package", ROOT / "packaging/linux/build.py")
        native = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(native)
        def readelf(*args, **kwargs):
            self.assertEqual(args[0], "readelf")
            name = Path(args[-1]).name
            if name == "lmm-extensions":
                return ""
            loader = ("[Requesting program interpreter: /lib64/ld-linux-x86-64.so.2]\n"
                      if name == "lmm-extore-host" else "")
            return loader + "(NEEDED) [libc.so.6]\n(NEEDED) [libssl.so.3]\nGLIBC_2.36 OPENSSL_3.0.0\n"
        digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        with patch.object(native, "run", side_effect=readelf):
            config, info = native.prepare(source, archive, digest, SHA, "extensions", "deb", 1,
                                          self.root / "deb-stage")
        self.assertEqual(info["extore_plugin"], record)
        self.assertEqual({p for p in info["files"] if p.startswith("/usr/bin/")}, {"/usr/bin/lmm-extensions"})
        self.assertEqual(info["files"]["/usr/libexec/lmm-extore-host"]["mode"], 0o755)
        self.assertEqual(info["files"]["/usr/lib/lmm/libextore_zplugin.so"]["mode"], 0o644)
        self.assertIn("/usr/share/licenses/lmm-extensions/extore/LICENSE", info["files"])
        self.assertIn("libc6 (>= 2.36)", info["dependencies"])
        self.assertIn("libssl3 (>= 3.0.0) | libssl3t64 (>= 3.0.0)", info["dependencies"])
        self.assertEqual(config["scripts"], {})
        self.assertEqual(config["name"], "lmm-extensions")
        with self.assertRaisesRegex(ValueError, "GNU Extore pair"):
            native.prepare(source, archive, digest, SHA, "extensions", "apk", 1, self.root / "apk-stage")
        library = self.inputs / "libextore_zplugin.so"
        with patch.object(native, "run", return_value="(NEEDED) [libmystery.so]\nGLIBC_2.36"):
            with self.assertRaisesRegex(ValueError, "unsupported"):
                native.inspect_elf(library, "amd64", shared=True)
        with patch.object(native, "run", return_value="(NEEDED) [libc.so.6]\nGLIBC_2.36"):
            with self.assertRaisesRegex(ValueError, "no executable interpreter"):
                native.inspect_elf(library, "amd64")
            self.assertEqual(native.inspect_elf(library, "amd64", shared=True)["linkage"], "glibc")


if __name__ == "__main__":
    unittest.main()
