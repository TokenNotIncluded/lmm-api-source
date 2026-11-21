#!/usr/bin/env python3
"""Synthetic authorization/collision cases; no customer facts or fake approval."""
from copy import deepcopy
import hashlib
import unittest

import migrate_current_identity as M


PASSWORD = "$2b$04$" + "a" * 53


def user(uid, **changes):
    row = {"id": uid, "username": "synthetic-user-" + str(uid), "display_name": "Synthetic",
           "email": "fixture" + str(uid) + "@example.test", "setting": "{}", "created_at": 1700000000,
           "auth_version": 1, "role": 1, "status": 1, "deleted_at": None, "password": PASSWORD,
           "trust_level_override": 0, "console_activated_at": 0, "github_id": "", "linux_do_id": "",
           "discord_id": "", "telegram_id": "", "oidc_id": "", "wechat_id": ""}
    row.update(changes)
    return row


def source_object(users, archive="b" * 64):
    data = {"users": users, "two_fas": [], "options": [{"key": "PasswordLoginEnabled", "value": "true"}]}
    records = M.mapped_current(data, archive)
    return {"format": M.SOURCE_FORMAT, "status": "SOURCE_IDENTITY_FACTS_NOT_REVIEW_NOT_APPROVAL",
            "original_source_columns_complete": False, "financial_approval": False,
            "raw_api_keys_selected": False, "client_secret_selected": False, "production_writes": 0,
            "query_exit_code": 0, "selected_columns": list(M.SOURCE_COLUMNS), "data": data,
            "source_archive_sha256": archive, "source_identity_facts_sha256": M.digest(M.identity_facts(data)),
            "current_policy_records": records}


def inputs():
    original_users = [user(1, role=100, github_id="synthetic-root-github"), user(2, status=2, password=""),
                      user(3), user(4, password="unsupported-legacy-value")]
    old = M.I.map_identity(original_users, [], source_sha256="a" * 64)
    current_users = deepcopy(original_users)
    current_users[1].update(status=1, auth_version=2)
    current_users[2]["trust_level_override"] = 1
    current_users += [user(5), user(6, password=""), user(8)]
    source = source_object(current_users)
    provider = {**M.O.GOOGLE_SOURCE_CONTRACT, "id": 1, "name": "Google", "enabled": True,
                "client_id": "synthetic-google-client", "scopes": "openid profile email", "auth_style": 0,
                "access_policy": None}
    oauth_data = {"users": [{k: u[k] for k in ("id", "github_id", "linux_do_id", "discord_id", "telegram_id")}
                            for u in current_users], "oauth_providers": [provider],
                  "oauth_bindings": [{"id": n, "user_id": uid, "provider_id": 1,
                                      "provider_user_id": "synthetic-google-sub-" + str(uid)}
                                     for n, uid in enumerate((1, 2, 6), 1)]}
    oauth_rows = [{"issuer": issuer, "subject": subject, "source_user_id": uid, "source_family": family}
                  for issuer, subject, uid, family in M.O.identity_oauth_rows(oauth_data)]
    projection = {k: [] for k in ("users", "accounts", "profiles", "passwords", "developer", "oauth",
                  "account_policies", "original_imports", "history_imports", "current_identity_imports", "routing_grants")}
    projection["active_routing_groups"] = ["default"]
    for r in old["records"]:
        uid, aid = r["user_id"], r["account_id"]
        projection["users"].append({"id": uid, "personal_account_id": aid, "platform_role": r["platform_role"],
                                    "active": r["active"], "auth_version": r["auth_version"]})
        projection["accounts"].append({"id": aid, "kind": "personal", "service_level": 0, "active": r["active"],
                                      "created_at": "2023-11-14T22:13:20+00:00"})
        projection["profiles"].append({**{k: r[k] for k in M.PROFILE_COLUMNS if k != "user_id"}, "user_id": uid})
        projection["developer"].append({"user_id": uid, "granted": r["developer_access_granted"],
                                       "source_reference": r["developer_access_source_reference"], "revision": 1})
        projection["account_policies"].append({"account_id": aid, "preference": "wallet_only", "revision": 1})
        if r["password_hash"] is not None:
            projection["passwords"].append({"user_id": uid, "login_name": "legacy-" + str(uid),
                "password_sha256": hashlib.sha256(r["password_hash"].encode()).hexdigest()})
    # A separately activated privileged native identity has a later version.
    projection["users"][0].update(active=True, auth_version=3)
    projection["accounts"][0]["active"] = True
    projection["oauth"].append({"user_id": 1, "issuer": "https://github.com", "subject": "synthetic-root-github"})
    manifest = {"scope": "identity-only", "mapping_sha256": old["mapping_sha256"],
                "mapping_file_sha256": hashlib.sha256((M.canonical(old) + "\n").encode()).hexdigest(),
                "source_target_id_map_sha256": hashlib.sha256(M.I.id_map_bytes(old["records"]).encode()).hexdigest()}
    projection["original_imports"].append({"migration_id": "synthetic-original", "source_archive_sha256": "a" * 64,
                                          "mapping_sha256": old["mapping_sha256"], "manifest": manifest})
    h = current_users[-1]
    facts = {k: h[k] for k in ("id", "role", "status", "username", "created_at", "deleted_at", "display_name")}
    projection["history_imports"].append({"migration_id": "synthetic-history", "manifest": {
        "scope": "history-only-identity-supplement.v1", "source_user_facts": [{"facts": facts, "source_fact_sha256": M.digest(facts)}],
        "mapping": [{"source_user_id": 8, "target_user_id": 9, "target_personal_account_id": 10}],
        "target_policy": {"users_active": False, "accounts_active": False, "credentials_imported": False, "password_login_allowed": False}}})
    projection["users"].append({"id": 9, "personal_account_id": 10, "platform_role": "user", "active": False, "auth_version": 1})
    projection["accounts"].append({"id": 10, "kind": "personal", "service_level": 0, "active": False,
                                  "created_at": "2023-11-14T22:13:20+00:00"})
    projection["profiles"].append({"user_id": 9, "username": h["username"], "display_name": h["display_name"], "email": "",
        "email_login_allowed": False, "setting": "{}", "password_login_allowed": False, "second_factor_required": False, "legacy_bcrypt_allowed": False})
    projection["account_policies"].append({"account_id": 10, "preference": "wallet_first", "revision": 3})
    projection["users"].append({"id": 20, "personal_account_id": 21, "platform_role": "user", "active": True, "auth_version": 7})
    projection["accounts"].append({"id": 21, "kind": "personal", "service_level": 4, "active": True,
                                  "created_at": "2024-01-01T00:00:00+00:00"})
    seq = {name: {"last_value": value, "is_called": False, "increment": 1, "cache": 1, "cycle": False, "max": M.MAX_ID}
           for name, value in zip(M.SEQUENCES, (40, 30))}
    bundle = {"kind": "synthetic-test-facts", "source": source, "oauth": {"projection": oauth_data, "oauth_records": oauth_rows},
              "current": source["current_policy_records"], "original": old, "oauth_rows": oauth_rows,
              "namespace": {"system_identifier": "1234", "database": "synthetic_source", "schema": "legacy"}, "files": {}}
    route_facts = {"users": [{**{k: u[k] for k in M.ROUTING_USER_COLUMNS if k != "group"}, "group": "default"}
                              for u in current_users], "options": [
        {"key": "GroupGroupRatio", "value": "{}"}, {"key": "GroupRatio", "value": '{"default":1}'},
        {"key": "UserUsableGroups", "value": '{"default":"Default"}'},
        {"key": "group_ratio_setting.group_special_usable_group", "value": "{}"}]}
    M.attach_routing(bundle, route_facts)
    return bundle, projection, seq


class IdentityPlanning(unittest.TestCase):
    def test_realistic_used_target_operations(self):
        b, p, s = inputs()
        result = M.plan_operations(b, p, s)
        mapping = {r["source_user_id"]: r for r in result["mapping"]}
        self.assertEqual((mapping[5]["target_user_id"], mapping[6]["target_personal_account_id"]), (30, 41))
        self.assertEqual(mapping[8]["target_user_id"], 9)
        self.assertEqual(result["new_user_count"], 2)
        self.assertEqual(result["history_completion_count"], 1)
        self.assertEqual(result["unsupported_password_without_mapped_method"], [4])
        ops = result["operations"]
        self.assertFalse(any(o["table"] in ("core_identity.users", "core_identity.accounts") and o["key"] == 1 for o in ops))
        self.assertFalse(any(o["table"] == "core_billing.account_policies" and o["key"] == 10 for o in ops))
        self.assertEqual(sum(o["table"] == "core_identity.oauth_identities" for o in ops), 3)
        history = next(o for o in ops if o["table"] == "core_identity.users" and o["key"] == 9)
        self.assertEqual(history["after"]["auth_version"], 2)
        self.assertTrue(history["after"]["active"])
        self.assertTrue(all(o["after"].get("email_login_allowed") is not True for o in ops))

    def test_subject_owner_collision(self):
        b, p, s = inputs()
        p["oauth"].append({"issuer": "https://accounts.google.com", "subject": "synthetic-google-sub-2", "user_id": 20})
        with self.assertRaisesRegex(M.Error, "subject_owner_collision"):
            M.plan_operations(b, p, s)

    def test_owner_has_other_google_subject(self):
        b, p, s = inputs()
        p["oauth"].append({"issuer": "https://accounts.google.com", "subject": "other-subject", "user_id": 2})
        with self.assertRaisesRegex(M.Error, "owner_has_other_subject"):
            M.plan_operations(b, p, s)

    def test_history_same_id_without_same_identity_is_rejected(self):
        b, p, s = inputs()
        fact = p["history_imports"][0]["manifest"]["source_user_facts"][0]
        fact["facts"]["created_at"] += 1
        fact["source_fact_sha256"] = M.digest(fact["facts"])
        with self.assertRaisesRegex(M.Error, "history_source_identity_unproven"):
            M.plan_operations(b, p, s)

    def test_preserve_independent_native_password_reset(self):
        b, p, s = inputs()
        p["passwords"][1]["password_sha256"] = "c" * 64
        with self.assertRaisesRegex(M.Error, "native_password_changed"):
            M.plan_operations(b, p, s)

    def test_do_not_override_independent_native_ordinary_authority(self):
        b, p, s = inputs()
        p["users"][1]["auth_version"] = 9
        with self.assertRaisesRegex(M.Error, "native_authority_diverged"):
            M.plan_operations(b, p, s)

    def test_do_not_override_independent_native_developer_grant(self):
        b, p, s = inputs()
        p["developer"][2]["source_reference"] = "native-reviewed-decision"
        with self.assertRaisesRegex(M.Error, "developer_authority_diverged"):
            M.plan_operations(b, p, s)

    def test_do_not_move_sequences_behind_native_ids(self):
        b, p, s = inputs()
        s[M.SEQUENCES[1]]["last_value"] = 10
        with self.assertRaisesRegex(M.Error, "sequence_behind_existing_ids"):
            M.plan_operations(b, p, s)

    def test_shared_cached_sequences_not_safe_for_preview(self):
        b, p, s = inputs()
        s[M.SEQUENCES[1]]["cache"] = 5
        with self.assertRaisesRegex(M.Error, "sequence_not_safe_monotonic"):
            M.plan_operations(b, p, s)

    def test_password_projection_tamper_is_rejected(self):
        b, p, s = inputs()
        b["source"]["current_policy_records"][0]["password_login_allowed"] = True
        with self.assertRaisesRegex(M.Error, "policy_mapping_changed"):
            M.plan_operations(b, p, s)

    def test_canonical_google_contract_not_slug_only(self):
        b, _, _ = inputs()
        b["oauth"]["projection"]["oauth_providers"][0]["token_endpoint"] = "https://attacker.example/token"
        with self.assertRaises(Exception):
            M.validate_oauth_projection(b["source"], b["oauth"])

    def test_new_mfa_is_not_silently_activated(self):
        b, p, s = inputs()
        b["source"]["data"]["two_fas"] = [{"user_id": 5, "is_enabled": True, "deleted_at": None}]
        b["source"]["source_identity_facts_sha256"] = M.digest(M.identity_facts(b["source"]["data"]))
        b["source"]["current_policy_records"] = M.mapped_current(b["source"]["data"], b["source"]["source_archive_sha256"])
        with self.assertRaisesRegex(M.Error, "new_mfa_owner"):
            M.plan_operations(b, p, s)

    def test_source_allowed_group_must_be_enabled_in_current_native_release(self):
        b, p, s = inputs()
        p["active_routing_groups"] = []
        with self.assertRaisesRegex(M.Error, "not_active_native"):
            M.plan_operations(b, p, s)

    def test_history_native_group_revocation_is_not_reversed(self):
        b, p, s = inputs()
        p["routing_grants"].append({"user_id": 9, "group_name": "default", "enabled": False,
                                     "source_reference": "independent-native-review"})
        with self.assertRaisesRegex(M.Error, "independently_disabled"):
            M.plan_operations(b, p, s)


if __name__ == "__main__":
    unittest.main()
