"""No database/network: production Google source-contract and identity planning checks."""
import copy
import unittest
from unittest.mock import patch

import legacy_cutover_import as importer


def user(identifier, username="existing.email@example.test"):
    return dict(id=identifier, username=username, display_name="Existing name",
                email=username, role=1, status=1, deleted_at=None, password="",
                auth_version=1, setting="{}", created_at=1700000000)


def source():
    # Independent copy of the confirmed non-secret production contract. Do
    # not derive this positive fixture from the implementation's allowlist.
    provider = dict(id=1, name="google", slug="google", enabled=True,
                    authorization_endpoint="https://accounts.google.com/o/oauth2/v2/auth",
                    token_endpoint="https://oauth2.googleapis.com/token",
                    user_info_endpoint="https://openidconnect.googleapis.com/v1/userinfo",
                    well_known="https://accounts.google.com/.well-known/openid-configuration",
                    user_id_field="sub", username_field="email",
                    display_name_field="name", email_field="email",
                    client_id="synthetic.apps.googleusercontent.com",
                    scopes="openid profile email", auth_style=0, access_policy="")
    return dict(users=[user(71), user(72, "other@example.test")], two_fas=[], options={},
                oauth_providers=[provider], oauth_bindings=[
                    dict(id=1, provider_id=1, user_id=71,
                         provider_user_id="100000000000000000001")])


class GoogleIdentityImportTests(unittest.TestCase):
    def rejected(self, data, code):
        with self.assertRaisesRegex(SystemExit, "error: " + code + "$"):
            importer.plan_identity(data, "a" * 64)

    def test_verified_google_origin_keeps_exact_subject_owner_and_existing_username(self):
        data = source()
        before = copy.deepcopy(data)
        statements, stats = importer.plan_identity(data, "a" * 64)
        oauth = [s for s in statements if s.startswith("INSERT INTO core_identity.oauth_identities")]
        self.assertEqual(oauth, [
            "INSERT INTO core_identity.oauth_identities(issuer,subject,user_id) "
            "VALUES('https://accounts.google.com','100000000000000000001',71);"])
        profiles = [s for s in statements if s.startswith("INSERT INTO core_identity.user_profiles")]
        self.assertIn("existing.email@example.test", profiles[0])
        self.assertNotIn("g_", profiles[0])
        self.assertEqual(stats["oauth_google"], 1)
        self.assertEqual(data, before)

    def test_every_google_endpoint_and_mapping_must_match_not_just_slug(self):
        for key in ("slug", "authorization_endpoint", "token_endpoint", "user_info_endpoint",
                    "well_known", "user_id_field", "username_field", "display_name_field", "email_field"):
            with self.subTest(key=key):
                data = source()
                data["oauth_providers"][0][key] += ".untrusted"
                self.rejected(data, "google_provider_source_contract_mismatch")
        for url in ["http://openidconnect.googleapis.com/v1/userinfo",
                    "https://openidconnect.googleapis.com.evil.test/v1/userinfo",
                    "https://openidconnect.googleapis.com@evil.test/v1/userinfo",
                    "https://openidconnect.googleapis.com/v1/userinfo?alternate=1",
                    "https://openidconnect.googleapis.com/v1/userinfo#fragment"]:
            with self.subTest(url=url):
                data = source()
                data["oauth_providers"][0]["user_info_endpoint"] = url
                self.rejected(data, "google_provider_source_contract_mismatch")

    def test_policy_auth_style_scopes_client_and_enabled_contract_fail_closed(self):
        for key, value in [("auth_style", 1), ("auth_style", False),
                           ("access_policy", '{"logic":"AND"}'),
                           ("scopes", "profile email"), ("scopes", "openid profile email extra"),
                           ("client_id", ""), ("client_id", "client\n"), ("enabled", False)]:
            with self.subTest(key=key, value=value):
                data = source()
                data["oauth_providers"][0][key] = value
                self.rejected(data, "google_provider_source_contract_mismatch")

    def test_bindings_cannot_be_lost_through_missing_provider_or_owner_join(self):
        for key, value, error in [("provider_id", 99, "oauth_binding_provider_missing"),
                                  ("provider_id", True, "oauth_binding_provider_missing"),
                                  ("user_id", 99, "oauth_binding_user_missing")]:
            with self.subTest(key=key, value=value):
                data = source()
                data["oauth_bindings"][0][key] = value
                self.rejected(data, error)
        data = source()
        del data["oauth_providers"]
        self.rejected(data, "oauth_provider_source_missing")

    def test_source_ownership_collision_and_duplicate_are_explicit_errors(self):
        for owner, error in [(72, "oauth_subject_owner_collision"), (71, "oauth_subject_duplicate")]:
            data = source()
            data["oauth_bindings"].append(dict(data["oauth_bindings"][0], id=2, user_id=owner))
            self.rejected(data, error)
        data = source()
        data["users"][0]["github_id"] = "42"
        data["users"][1]["github_id"] = "42"
        self.rejected(data, "oauth_subject_owner_collision")

    def test_provider_and_binding_source_ids_cannot_be_ambiguous(self):
        data = source()
        data["oauth_providers"].append(dict(data["oauth_providers"][0]))
        self.rejected(data, "oauth_provider_source_duplicate")
        data = source()
        data["oauth_bindings"].append(dict(data["oauth_bindings"][0]))
        self.rejected(data, "oauth_binding_source_invalid")

    def test_subjects_never_truncate_trim_or_coerce_numbers(self):
        for value, error in [("x" * 256, "oauth_subject_too_long"),
                             ("é" * 128, "oauth_subject_too_long"),
                             (" 123", "oauth_subject_invalid"), ("123 ", "oauth_subject_invalid"),
                             ("123\x00", "oauth_subject_invalid"), (123, "oauth_subject_invalid"),
                             ("", "oauth_subject_invalid")]:
            with self.subTest(value=repr(value)):
                data = source()
                data["oauth_bindings"][0]["provider_user_id"] = value
                self.rejected(data, error)
        data = source()
        data["oauth_bindings"][0]["provider_user_id"] = "é" * 127 + "x"
        rows = importer.identity_oauth_rows(data)
        self.assertEqual(rows[0][1], "é" * 127 + "x")

    def test_builtin_providers_also_reject_overlong_subjects_without_truncation(self):
        for column in importer.OAUTH_ISSUERS:
            with self.subTest(column=column):
                data = source()
                data["users"][0][column] = "x" * 256
                self.rejected(data, "oauth_subject_too_long")

    def test_all_production_sized_google_bindings_keep_exact_owner_mapping(self):
        data = source()
        data["users"] = [user(i, f"user{i}@example.test") for i in range(1, 247)]
        data["oauth_bindings"] = [dict(id=i, provider_id=1, user_id=i,
            provider_user_id=str(10**20+i)) for i in range(1, 247)]
        rows = importer.identity_oauth_rows(data)
        self.assertEqual(len(rows), 246)
        self.assertEqual({(r[1], r[2]) for r in rows}, {(str(10**20+i), i) for i in range(1, 247)})

    def test_builtin_and_custom_identity_sql_never_swallows_target_conflicts(self):
        data = source()
        for column in importer.OAUTH_ISSUERS:
            data["users"][0][column] = "42"
        statements, stats = importer.plan_identity(data, "a" * 64)
        oauth = [s for s in statements if s.startswith("INSERT INTO core_identity.oauth_identities")]
        self.assertEqual(len(oauth), 4)
        self.assertTrue(all("ON CONFLICT" not in s for s in oauth))
        self.assertNotIn("oauth_unsupported_provider", stats)

    def test_source_loader_retains_provider_ids_and_never_reads_client_secrets(self):
        class Reader:
            def __init__(self):
                self.queries = []

            def run(self, statement):
                self.queries.append(statement)
                return "[]"

        reader = Reader()
        with patch.object(importer.KEYS, "load_source", return_value={}):
            loaded = importer.load_source(reader, "legacy")
        binding = next(q for q in reader.queries if "FROM user_oauth_bindings" in q)
        provider = next(q for q in reader.queries if "FROM custom_oauth_providers" in q)
        self.assertIn("provider_id,provider_user_id", binding)
        self.assertNotIn("JOIN", binding)
        self.assertNotIn("client_secret", provider)
        self.assertIn("user_id_field,username_field", provider)
        self.assertIn("oauth_providers", loaded)


if __name__ == "__main__":
    unittest.main()
