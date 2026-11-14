"""The fresh69 source review must cover the toolchain and Cargo examples."""
from pathlib import Path
from tempfile import TemporaryDirectory
import unittest

import financial_import as F


class SuccessorClosureTests(unittest.TestCase):
    def test_toolchain_examples_and_external_modules_cannot_be_omitted(self):
        names = (
            "services/core/Cargo.toml", "services/core/Cargo.lock",
            "services/core/build.rs", "services/core/rust-toolchain.toml",
            "mise.toml", "deployment/postgres/grant-core-runtime.sql",
            "contracts/assistant-tools-v1.json", "contracts/core/v1/funding-cases.json",
            "services/extensions/internal/modules/payments/schema.sql",
            "services/extensions/internal/modules/store/schema.sql",
            "services/extensions/internal/modules/toolmarket/builtin_catalog.json",
            "services/core/examples/protocol-events-fixture.rs",
            "tests/model_process/resource_cases.rs",
        )
        with TemporaryDirectory() as directory:
            root = Path(directory)
            for name in names:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("fixture")
            files = dict.fromkeys(names, "0" * 64)
            F.IDENTITY.require_successor69_source_closure(root, files)
            for missing in ("services/core/rust-toolchain.toml",
                            "services/core/examples/protocol-events-fixture.rs",
                            "tests/model_process/resource_cases.rs"):
                with self.subTest(missing=missing):
                    incomplete = {name: digest for name, digest in files.items()
                                  if name != missing}
                    with self.assertRaisesRegex(F.IDENTITY.MappingError, "source_closure_incomplete"):
                        F.IDENTITY.require_successor69_source_closure(root, incomplete)


if __name__ == "__main__":
    unittest.main()
