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


def inputs(*, privileged=True, old_developer=False, current_developer=True):
    original_users = [user(1, role=100 if privileged else 1, github_id="synthetic-root-github"), user(2, status=2, password=""),
                      user(3, trust_level_override=int(old_developer)), user(4, password="unsupported-legacy-value")]
    old = M.I.map_identity(original_users, [], source_sha256="a" * 64)
    current_users = deepcopy(original_users)
    current_users[1].update(status=1, auth_version=2)
    current_users[2]["trust_level_override"] = int(current_developer)
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
    projection["defined_routing_groups"] = ["default"]
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
    if privileged:
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
        "target_policy": {"users_active": False, "accounts_active": False, "credentials_imported": False,
                          "password_login_allowed": False, "auth_version": 1}}})
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
        self.assertEqual(history["after"]["auth_version"], 3)
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

    def test_only_active_change_has_one_native_generation(self):
        b, p, s = inputs()
        plan = M.plan_operations(b, p, s)
        stage = next(r for r in plan["auth_version_lineage"] if r["source_user_id"] == 2)
        self.assertEqual((stage["source_auth_version"], stage["native_before"]["auth_version"],
                          stage["explicit_user_auth_version"], stage["profile_policy_trigger_increment"],
                          stage["planned_final_auth_version"]), (2, 1, 2, 0, 2))

    def test_only_profile_policy_change_uses_trigger_once(self):
        b, p, s = inputs(privileged=False, current_developer=False)
        b["source"]["data"]["options"][0]["value"] = "false"
        b["source"]["source_identity_facts_sha256"] = M.digest(M.identity_facts(b["source"]["data"]))
        b["source"]["current_policy_records"] = M.mapped_current(b["source"]["data"], b["source"]["source_archive_sha256"])
        plan = M.plan_operations(b, p, s)
        stage = next(r for r in plan["auth_version_lineage"] if r["source_user_id"] == 3)
        self.assertEqual((stage["source_auth_version"], stage["explicit_user_auth_version"],
                          stage["profile_policy_trigger_increment"], stage["planned_final_auth_version"]), (1, 1, 1, 2))
        op = next(o for o in plan["operations"] if o["table"] == "core_identity.users" and o["key"] == 3)
        self.assertTrue(op["before"]["active"] and op["after"]["active"])

    def test_history_active_and_profile_policy_are_two_events(self):
        b, p, s = inputs()
        plan = M.plan_operations(b, p, s)
        stage = next(r for r in plan["auth_version_lineage"] if r["source_user_id"] == 8)
        self.assertEqual((stage["source_auth_version"], stage["native_before"]["auth_version"],
                          stage["explicit_user_auth_version"], stage["profile_policy_trigger_increment"],
                          stage["planned_final_auth_version"]), (1, 1, 2, 1, 3))
        self.assertEqual(stage["binding_kind"], "reviewed_history_completion")

    def test_developer_revocation_has_its_own_trigger_increment(self):
        b, p, s = inputs(old_developer=True, current_developer=False)
        plan = M.plan_operations(b, p, s)
        stage = next(r for r in plan["auth_version_lineage"] if r["source_user_id"] == 3)
        self.assertEqual((stage["source_auth_version"], stage["explicit_user_auth_version"],
                          stage["profile_policy_trigger_increment"], stage["developer_revocation_trigger_increment"],
                          stage["planned_final_auth_version"]), (1, 1, 0, 1, 2))

    def test_disable_profile_and_developer_revocation_are_all_preserved(self):
        b, p, s = inputs(old_developer=True, current_developer=False)
        row = next(r for r in b["source"]["data"]["users"] if r["id"] == 3)
        row.update(status=2, auth_version=2)
        b["source"]["source_identity_facts_sha256"] = M.digest(M.identity_facts(b["source"]["data"]))
        b["source"]["current_policy_records"] = M.mapped_current(b["source"]["data"], b["source"]["source_archive_sha256"])
        for route in b["routing"]["facts"]["users"]:
            if route["id"] == 3:
                route.update(status=2, auth_version=2)
        M.attach_routing(b, b["routing"]["facts"])
        plan = M.plan_operations(b, p, s)
        stage = next(r for r in plan["auth_version_lineage"] if r["source_user_id"] == 3)
        self.assertEqual((stage["explicit_user_auth_version"], stage["profile_policy_trigger_increment"],
                          stage["developer_revocation_trigger_increment"], stage["planned_final_auth_version"]), (2, 1, 1, 4))

    def test_independent_native_generation_without_source_drift_is_rejected(self):
        b, p, s = inputs(current_developer=False)
        p["users"][2]["auth_version"] += 1
        with self.assertRaisesRegex(M.Error, "native_authority_diverged"):
            M.plan_operations(b, p, s)

    def test_history_independent_generation_is_not_source_completion_authority(self):
        b, p, s = inputs()
        next(r for r in p["users"] if r["id"] == 9)["auth_version"] = 2
        with self.assertRaisesRegex(M.Error, "history_independent_native_generation_drift"):
            M.plan_operations(b, p, s)

    def test_privileged_and_unrelated_native_generations_are_preserved(self):
        b, p, s = inputs()
        plan = M.plan_operations(b, p, s)
        stage = next(r for r in plan["auth_version_lineage"] if r["source_user_id"] == 1)
        self.assertEqual((stage["source_auth_version"], stage["native_before"]["auth_version"],
                          stage["planned_final_auth_version"]), (1, 3, 3))
        self.assertEqual(stage["binding_kind"], "preserved_privileged_requires_separate_lineage")
        self.assertFalse(any(o["table"] == "core_identity.users" and o["key"] in (1, 20) for o in plan["operations"]))

    def test_dormant_default_retains_membership_with_explicit_zero_capability(self):
        b, p, s = inputs()
        p["active_routing_groups"] = p["defined_routing_groups"] = []
        M.attach_capabilities(b, {"abilities": [], "channels": []})
        plan = M.plan_operations(b, p, s)
        self.assertEqual(len(plan["routing_grant_proofs"]), 3)
        self.assertTrue(all(r["native_availability"]["kind"] == "dormant_source_own_default_membership"
                            for r in plan["routing_grant_proofs"]))
        grants = [o for o in plan["operations"] if o["table"] == "core_identity.user_routing_groups"]
        self.assertEqual({o["after"]["group_name"] for o in grants}, {"default"})
        self.assertTrue(all(o["after"]["enabled"] for o in grants))

    def test_dormant_default_cannot_override_a_native_disabled_group(self):
        b, p, s = inputs()
        p["active_routing_groups"] = []
        M.attach_capabilities(b, {"abilities": [], "channels": []})
        with self.assertRaisesRegex(M.Error, "not_active_native"):
            M.plan_operations(b, p, s)

    def test_disabled_source_default_channel_still_blocks_dormant_exception(self):
        b, p, s = inputs()
        p["active_routing_groups"] = p["defined_routing_groups"] = []
        M.attach_capabilities(b, {"abilities": [], "channels": [{"id": 1, "type": 1, "status": 2,
            "group": "default", "models": "synthetic-model", "priority": 0, "weight": 1, "model_mapping": ""}]})
        with self.assertRaisesRegex(M.Error, "dormant_default_has_source_capability"):
            M.plan_operations(b, p, s)

    def test_disabled_source_default_ability_still_blocks_dormant_exception(self):
        b, p, s = inputs()
        p["active_routing_groups"] = p["defined_routing_groups"] = []
        M.attach_capabilities(b, {"abilities": [{"group": "default", "model": "synthetic-model", "channel_id": 1,
            "enabled": False, "priority": 0, "weight": 1, "tag": None}], "channels": [{"id": 1, "type": 1, "status": 2,
            "group": "other", "models": "synthetic-model", "priority": 0, "weight": 1, "model_mapping": ""}]})
        with self.assertRaisesRegex(M.Error, "dormant_default_has_source_capability"):
            M.plan_operations(b, p, s)

    def test_source_capability_change_at_fence_is_rejected(self):
        # This tests fact comparison only; it does not fabricate a barrier,
        # verification receipt, external review or production authorization.
        b, p, s = inputs()
        p["active_routing_groups"] = p["defined_routing_groups"] = []
        M.attach_capabilities(b, {"abilities": [], "channels": []})
        plan = M.plan_operations(b, p, s)
        facts = {"identity": M.identity_facts(b["source"]["data"]),
                 "oauth": {k: b["oauth"]["projection"][k] for k in ("users", "oauth_providers", "oauth_bindings")},
                 "routing": b["routing"]["facts"], "routing_capabilities": b["routing_capabilities"]["facts"]}
        material = {"plan": plan, "source_identity_facts_sha256": M.digest(facts["identity"]),
                    "source_oauth_facts_sha256": M.digest(facts["oauth"]),
                    "source_routing_facts_sha256": M.digest(facts["routing"]),
                    "source_routing_capability_facts_sha256": M.digest(facts["routing_capabilities"])}
        M.validate_observed_source_facts(material, facts)
        changed = deepcopy(facts)
        changed["routing_capabilities"]["channels"].append({"id": 1, "type": 1, "status": 2,
            "group": "default", "models": "synthetic-model", "priority": 0, "weight": 1, "model_mapping": ""})
        with self.assertRaisesRegex(M.Error, "capabilities_changed_after_preview"):
            M.validate_observed_source_facts(material, changed)
        material["source_routing_capability_facts_sha256"] = M.digest(changed["routing_capabilities"])
        with self.assertRaisesRegex(M.Error, "dormant_default_has_source_capability"):
            M.validate_observed_source_facts(material, changed)


if __name__ == "__main__":
    unittest.main()
