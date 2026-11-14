#!/usr/bin/env python3
"""Offline source boundaries. Runtime, database and Docker tests are separate."""
import configparser
import hashlib
import json
import re
from pathlib import Path
import subprocess
import tomllib
import unittest

ROOT = Path(__file__).resolve().parents[1]
# Reviewed offline installer bodies are frozen independently of runtime pool
# ownership. A change requires re-review of the complete owner operation, not
# extending a runtime SQL allowlist. Comments/formatting do not change tokens.
OFFLINE_INSTALLER = "services/extensions/internal/freshinstall/install.go"
OFFLINE_COMMERCE_PLAN = "services/extensions/internal/freshinstall/commerce.go"
OFFLINE_CLI = "services/extensions/internal/freshinstall/cli.go"
OFFLINE_WALLET_CLI = "services/extensions/internal/walletupgrade/cli.go"
OFFLINE_WALLET_UPGRADE = "services/extensions/internal/walletupgrade/upgrade.go"
OFFLINE_OPERATIONS_CLI = "services/extensions/internal/freshinstall/operations_upgrade.go"
OFFLINE_RSS_UPGRADE = "services/extensions/internal/modules/operations/rss_upgrade.go"
OFFLINE_REPOSITORY_UPGRADE = "services/extensions/internal/modules/operations/script_repository_upgrade.go"
OFFLINE_MAIN = "services/extensions/cmd/extensions/main.go"
OFFLINE_SNAPSHOT = "services/extensions/internal/modules/toolmarket/install_snapshot.go"
OFFLINE_TOKEN_SHA256 = {
    # Closed store/support plan; BEGIN/COMMIT are stripped only after one-pair checks.
    OFFLINE_COMMERCE_PLAN: '97c1a80f76fae7a6bf08cc57ff3174f0ece4f5a67238bf7163fc26e0cbde39f2',
    # Reviewed 2026-11-14 against exact prior source 2e2ae58; semantic
    # role/recovery/CAS tests below stay independent of these fingerprints.
    # APP01 owner-only v2->v3: preserve predecessor grants/data, scrub new ACLs,
    # grant only repository SELECT/UPDATE and retain the original receipt.
    OFFLINE_REPOSITORY_UPGRADE: 'c67e86d23867a3ac03320123ed58457645e83572750c731ae404b2b46dc33eb4',
    # RSS still seals its intermediate v2. The CLI defaults to that target and
    # dispatches v3 explicitly, retaining the same owner/runtime writer fences
    # and the original operation's recovery identity.
    'services/extensions/internal/modules/operations/rss_upgrade.go': 'c31909390df3d4b1beea8a9ebaec909fa36c12059be698c62c36576ff3bdff57',
    'services/extensions/internal/freshinstall/operations_upgrade.go': 'ae7a11320bb66068daf63d1f6c34b37ab709ff8fde2c9cc5e9d4c1b2e3fd3aee',
    # Checkins/commerce and subscription schemas add compiled plans only;
    # optional payment/operations addons remain under complete ACL checks.
    # Community/acquisition use compiled schemas and dedicated runtime grants;
    # URL, owner, empty-database and transaction checks remain in place.
    'services/extensions/internal/freshinstall/install.go': 'e44d031e1e38344b36108f1f38edbc317e812f93f36e4872860cc9617170f9e2',
    OFFLINE_WALLET_CLI: '87cf633970571aef5b7abc5b3788c30e6bc62c77b1799815faa845f4e9c1ded7',
    OFFLINE_WALLET_UPGRADE: '923244ef59326a37b79ee2bd7a00132a492cf6e8535854356a77f92f7d092aba',
    'services/extensions/internal/freshinstall/cli.go': '014929777131a77c2fe599d53b90d7b65f7ccf6be7f51d4355cc57ac18dd2ae8',
    'services/extensions/cmd/extensions/main.go': 'c81a5a0b3e46f5da7e5f5c5f99ebea3ebed5fa34b5b071d3c187b2563104d6e7',
    'services/extensions/internal/modules/toolmarket/install_snapshot.go': '8f6f852068a5891a490876eb8918a58008d3f80844144f84307ba78c36b56013',
}
STORE_READ_QUERIES = {'services/extensions/internal/modules/store/promotions.go': ('QueryContext', "`SELECT body->'legacy'->>'promotion_id',\nCOUNT(*) FILTER (WHERE body->>'state' NOT IN ('awaiting_payment','payment_pending')),\nCOUNT(*) FILTER (WHERE body->>'state' IN ('awaiting_payment','payment_pending'))\nFROM lmm_store.orders WHERE shop_id=$1 AND body->'legacy'->>'product_id'=$2\nAND COALESCE(body->'legacy'->>'promotion_id','')<>'' AND body->>'state' NOT IN ('cancelled','refunded')\nGROUP BY body->'legacy'->>'promotion_id'`"), 'services/extensions/internal/modules/store/purchase_limits.go': ('QueryRowContext', "`SELECT COALESCE(SUM((o.body->>'quantity')::bigint),0)\nFROM lmm_store.orders o JOIN lmm_store.variants v ON v.shop_id=o.shop_id AND v.id=o.body->>'variant_id'\nWHERE o.shop_id=$1 AND v.body->>'product_id'=$2 AND o.body->'buyer'->>'kind'=$3\nAND (o.body->'buyer'->>'id')::bigint=$4 AND o.body->>'state' NOT IN ('cancelled','refunded')`")}

COMPOSITION_BUNDLE = "services/extensions/internal/composition/storage_bundle.go"
BUNDLE_POLICY_TOKEN_SHA256 = "51c9a41318b6aa6aaa319e99a0d1853f4bdf1cf86e6664bac7a8f5fd7a8df6a1"
STORAGE_TOKEN_SHA256 = "2aa8a755010114321cdcc6d836f77d266206914d065cee20d9dcbe024a60dfcc"
COMPOSITION_STORAGE = "services/extensions/internal/composition/storage.go"
COMPOSITION_PAYMENTS = "services/extensions/internal/composition/payments.go"
COMPOSITION_ASSISTANT = "services/extensions/internal/composition/assistant.go"
OPERATIONS_POSTGRES = "services/extensions/internal/modules/operations/postgres.go"
OPERATIONS_RELEASES = "services/extensions/internal/modules/operations/releases.go"
OPERATIONS_INJECTION = "services/extensions/internal/modules/operations/scripts.go"
OPERATIONS_RSS = "services/extensions/internal/modules/operations/rss.go"
OPERATIONS_REPOSITORY = "services/extensions/internal/modules/operations/script_repository.go"
OPERATIONS_REPOSITORY_FETCH = "services/extensions/internal/modules/operations/script_repository_fetch.go"
# Reviewed SQL contract, independent of the source-token fingerprints. Do not
# derive these expectations from the implementation or a passing run.
OPERATIONS_REPOSITORY_QUERIES = (
    ("Settings", "SELECT revision,repository_url,branch,snapshot_repository_url,snapshot_branch,snapshot_pulled_at,coalesce(to_jsonb(r)->>'github_key_ciphertext','')<>'' FROM lmm_operations.script_repository r WHERE singleton"),
    ("SaveWithCredentials", "UPDATE lmm_operations.script_repository AS r SET repository_url=$1,branch=$2,revision=revision+1,updated_by=$3 WHERE singleton AND revision=$4 RETURNING revision,snapshot_repository_url,snapshot_branch,snapshot_pulled_at,coalesce(to_jsonb(r)->>'github_key_ciphertext','')<>''"),
    ("SaveWithCredentials", "UPDATE lmm_operations.script_repository AS r SET repository_url=$1,branch=$2,revision=revision+1,updated_by=$3,github_key_ciphertext=$5 WHERE singleton AND revision=$4 RETURNING revision,snapshot_repository_url,snapshot_branch,snapshot_pulled_at,github_key_ciphertext<>''"),
    ("pull", "UPDATE lmm_operations.script_repository SET scripts=$1,snapshot_repository_url=repository_url,snapshot_branch=branch,snapshot_pulled_at=clock_timestamp(),updated_by=$2,revision=revision+1 WHERE singleton AND revision=$3 RETURNING revision,snapshot_repository_url,snapshot_branch,snapshot_pulled_at"),
    ("snapshot", "SELECT scripts,snapshot_pulled_at FROM lmm_operations.script_repository WHERE singleton"),
)

SQL_METHODS = {
    "Query", "QueryRow", "Exec", "QueryContext", "QueryRowContext", "ExecContext",
    "Begin", "BeginTx", "Prepare", "PrepareContext", "Conn", "Driver", "Raw",
}
MODULE_STORAGE_FILES = {
    "community": (
        "service.go", "acquisition_read.go", "directory.go", "directory_ad_payment.go",
        "onboarding.go", "profile_share.go", "signal.go",
    ),
    "acquisition": (
        "service.go", "activity.go", "corrections.go", "cost.go", "details.go",
        "export.go", "funnel.go", "links.go", "lookback.go", "observe.go",
        "report.go", "users.go", "visitors.go",
    ),
}
PQ_ARRAY_ADAPTERS = {
    "services/extensions/internal/modules/community/acquisition_read.go",
    "services/extensions/internal/modules/acquisition/visitors.go",
    "services/extensions/internal/modules/assistant_services/web_support.go",
}
# Exact file registrations, not directory or provider capability exemptions.
# These seals bind the reviewed source, while tables/predicates/pool checks below
# express the storage boundary independently. They are not runtime ACL receipts.
REVIEWED_BORROWED_SQL = {
    "services/extensions/internal/modules/assistant/agent_start_fence.go": {
        "namespace": "lmm_assistant",
        "tables": [
            "lmm_assistant.conversations",
            "lmm_assistant.delegation_conversations",
            "lmm_assistant.agent_turns"
        ],
        "predicates": [
            "WHERE id=$1 AND user_id=$2 FOR UPDATE",
            "WHERE user_id=$1 AND conversation_id=$2",
            "WHERE user_id=$1 AND turn_id=$2"
        ],
        "methods": [
            "BeginTx",
            "ExecContext",
            "QueryRowContext",
            "QueryRowContext",
            "QueryRowContext",
            "ExecContext"
        ],
        "sha256": "667ec9554cc1a2415f0d08702549eb05833d666e05125b9c67dba5fc2b8a9a49"
    },
    "services/extensions/internal/modules/payments/subscription_epay_postgres.go": {
        "namespace": "payment",
        "tables": [
            "payment_subscription_inbox"
        ],
        "predicates": [
            "WHERE evidence_id=$1 AND event_digest=$4 AND payload=$5",
            "WHERE evidence_id=$1 AND subscription_id IS NULL"
        ],
        "methods": [
            "ExecContext",
            "QueryRowContext",
            "QueryContext",
            "QueryRowContext",
            "ExecContext"
        ],
        "sha256": "16b9adf94f389862bc1b2eb1d65978671e2cfb8afa6a0478f2d4f7224ca2687d"
    },
    "services/extensions/internal/modules/payments/subscription_stripe.go": {
        "namespace": "payment",
        "tables": [
            "pg_class",
            "pg_namespace",
            "payment_stripe_subscription_checkouts",
            "payment_stripe_subscription_inbox"
        ],
        "predicates": [
            "WHERE order_id=$1 AND (checkout_id IS NULL OR",
            "WHERE evidence_id=$1 AND payload=$3",
            "WHERE evidence_id=$1 AND state<>'accepted'"
        ],
        "methods": [
            "ExecContext",
            "QueryRowContext",
            "QueryRowContext",
            "QueryContext",
            "QueryContext"
        ],
        "sha256": "c05200a79c5c69668728826b2859e26bc88f8e02c7dbb1cf6abdd508fa6bf654"
    },
    "services/extensions/internal/modules/payments/subscription_pancake.go": {
        "namespace": "payment",
        "tables": [
            "pg_class",
            "pg_namespace",
            "payment_pancake_subscription_checkouts",
            "payment_pancake_subscription_inbox"
        ],
        "predicates": [
            "WHERE order_id=$1 AND (checkout_id IS NULL OR",
            "WHERE evidence_id=$1 AND state<>'accepted'"
        ],
        "methods": [
            "QueryRowContext",
            "ExecContext",
            "QueryRowContext",
            "Query",
            "QueryContext",
            "QueryContext"
        ],
        "sha256": "fd882796f9fe30f2857e823d39368a79f9541b965a33c775fc8835921be768d6"
    },
    "services/extensions/internal/modules/payments/subscription_pancake_wallet_recovery.go": {
        "namespace": "payment",
        "tables": [
            "payment_pancake_wallet_inbox"
        ],
        "predicates": [
            "WHERE evidence_id=$1 AND state='pending'"
        ],
        "methods": [
            "QueryContext"
        ],
        "sha256": "6a4497188612f9cb150edf0c50dcb3820cb12f2633c67c396d36f190ff410435"
    },
    "services/extensions/internal/modules/store/postgres_legacy_pickup.go": {
        "namespace": "lmm_store",
        "tables": [
            "lmm_store.commands"
        ],
        "predicates": [
            "WHERE id=$1 AND body->>'kind'='legacy-store-pickup-v1' AND body->>'token_hash'=$2 LIMIT 2"
        ],
        "methods": [
            "QueryContext"
        ],
        "sha256": "3a987a70b8a1650761154073ca50bd33c94701ca2bd9672e1d5a0e5c74f2e534"
    },
    "services/extensions/internal/modules/store/postgres_pickup_mail.go": {
        "namespace": "lmm_store",
        "tables": [
            "lmm_store.commands"
        ],
        "predicates": [
            "AND (shop_id,id)>($1,$2)",
            "ORDER BY shop_id,id LIMIT $3"
        ],
        "methods": [
            "QueryContext"
        ],
        "sha256": "48b2f9c236071f3c41eb26862347388f88f1d812cff7fff3310706e67d506629"
    },
    "services/extensions/internal/modules/operations/script_repository_credentials.go": {
        "namespace": "lmm_operations",
        "tables": [
            "pg_attribute",
            "lmm_operations.script_repository"
        ],
        "predicates": [
            "WHERE singleton AND revision=$1"
        ],
        "methods": [
            "QueryRowContext",
            "QueryRowContext"
        ],
        "sha256": "51f436bda228bfd12727081aa91adba87b26c660abbc44e933e5e21cd81e5ba9"
    },
    "services/extensions/internal/modules/operations/ionet.go": {
        "namespace": "lmm_operations",
        "tables": [
            "lmm_operations.ionet_config"
        ],
        "predicates": [
            "WHERE singleton AND revision=$4 RETURNING revision"
        ],
        "methods": [
            "QueryRowContext",
            "QueryRowContext",
            "QueryRowContext"
        ],
        "sha256": "14b0b3fa3bdd67c98f40829ee5ef17dfa786b9bb8fae6bd2bc6922b75f232d2a"
    },
    "services/extensions/internal/modules/operations/ionet_journal.go": {
        "namespace": "lmm_operations",
        "tables": [
            "lmm_operations.ionet_operations",
            "lmm_operations.ionet_config"
        ],
        "predicates": [
            "WHERE actor_user_id=$1 AND operation_key=$2",
            "WHERE actor_user_id=$4 AND operation_key=$5 AND state='unknown'"
        ],
        "methods": [
            "QueryRowContext",
            "QueryRowContext",
            "ExecContext",
            "QueryContext",
            "ExecContext"
        ],
        "sha256": "8fd0ec7449018e4d24fb1107191d4e8944b64db97353285ac7d44cd077ea5687"
    },
    "services/extensions/internal/modules/community/navigation.go": {
        "namespace": "lmm_community",
        "tables": [
            "lmm_community.navigation"
        ],
        "predicates": [
            "WHERE singleton AND ($3::text IS NULL OR value=$3) RETURNING value,revision"
        ],
        "methods": [
            "QueryRowContext",
            "QueryRowContext"
        ],
        "sha256": "4147953ce184901d26951498515725c8bb315592114c43b13e871c77c4da4371"
    },
    "services/extensions/internal/modules/checkins/service.go": {
        "namespace": "lmm_checkins",
        "tables": [
            "lmm_checkins.contract",
            "lmm_checkins.active_policy",
            "lmm_checkins.policy_revisions",
            "lmm_checkins.intentions",
            "lmm_checkins.claim_commands"
        ],
        "predicates": [
            "WHERE account_id=$1 AND day=$2::date",
            "WHERE user_id=$1 AND operation_key=$2",
            "WHERE account_id=$2 AND day=$3::date AND state='pending'"
        ],
        "methods": [
            "QueryRowContext",
            "BeginTx",
            "ExecContext",
            "QueryRowContext",
            "QueryRowContext",
            "QueryContext",
            "QueryRowContext",
            "ExecContext",
            "QueryRowContext",
            "ExecContext",
            "ExecContext",
            "ExecContext",
            "ExecContext",
            "ExecContext"
        ],
        "sha256": "6f5f8001dae2333ce52d49ce8faa139f1976042cc12423072cc2a38884faa04d"
    }
}

# Exact offline owner closures qualified by actual PG18 CLI/ACL/replay evidence.
# EPay/Stripe original receipts and Pancake/checkin private probes were reviewed
# 2026-11-14. This grants no runtime SQL or provider/funds exception. Each CLI
# remains confined by OWNER_ENTRY_CALLERS; embedded schemas/pins are sealed too.
REVIEWED_OWNER_BOUNDARIES = {
    "services/extensions/internal/modules/operations/script_repository_credentials_upgrade.go": "d82054fbba76b66bd18137bf489d05aadfbaa9dfdc2bfe33e0f7ab1cb917dcd7",
    "services/extensions/internal/modules/operations/ionet_upgrade.go": "6eb4c17261ea53d121385bbdea0d01502e089ad58e48a3bf3e6d01e14be074ab",
    "services/extensions/internal/freshinstall/payments_upgrade.go": "213cdac0d35354206a4a0dff53b7ad8d52d31dd1b2e5b91e963c4ae57e56cb3c",
    "services/extensions/internal/freshinstall/checkin_policy.go": "530834845248e3a119442f2913103102ed1472cce2021075151b88b6b59cc2c2",
    "services/extensions/internal/modules/payments/subscription_epay_upgrade.go": "032e8bfa0774bf7001c761a28d666b48645e85b4786cb07206c4997cedd3bbf7",
    "services/extensions/internal/modules/payments/subscription_stripe_upgrade.go": "cd89af6c607eca9072726937588fc5365b52e63f50f10a42fbb2350d42e25690",
    "services/extensions/internal/modules/payments/subscription_pancake_upgrade.go": "3ace1caf899ee56e68f9d8c9711f3421b753da7c1ac01f6fd1f56e4376b1e3ed",
    "services/extensions/internal/modules/checkins/owner.go": "0736674e5c0939abf39da0f528e3d03818d22045473ae62174ed76dbc3dcec06"
}
REVIEWED_OWNER_INPUTS = {
    "services/extensions/internal/modules/operations/postgres.go": "6e2ad61c0f515461be2b70e97de829d0b8625157eceeaef25b7382bcef38e9ad",
    "services/extensions/internal/modules/operations/script_repository_credentials_schema.sql": "cb52890bcfa8fea138146523aa5f5d52d0cf10d0add067b6d239eb212f25ebd6",
    "services/extensions/internal/modules/operations/ionet_schema.sql": "c56821a0c8f7be66145125bacf943a67e0eb3e92726d89868ee33e7ca39921ac",
    "services/extensions/internal/modules/payments/schema.sql": "2c8575958e00b43ead3040dff527b0758c229dbe91278eaa552fe2e7a9cf8409",
    "services/extensions/internal/modules/payments/subscription_epay_schema.sql": "819faf495ef46f761970779fc2ff6012f499b0b1e399a22a9bf293f9df3741a6",
    "services/extensions/internal/modules/payments/subscription_epay_catalog.sql": "45ba53a1611a9eb71f38901f0125cc2d08d10a9f27cf2c0456c5120ac631af33",
    "services/extensions/internal/modules/payments/subscription_epay_storage_contract.json": "01e777483a88c00aca43987d86773abdb5fa7e5758d1c13c433dae6f8febcba4",
    "services/extensions/internal/modules/payments/subscription_stripe_schema.sql": "01f966266f339fa634db2f24875ebd6e49a56c91050d1f5df3f6d8369fd40aef",
    "services/extensions/internal/modules/payments/subscription_stripe_catalog.sql": "3eb72d3a99567067e8ae239213f626c6e2f6f35f06a1cd05eef390802e9e747c",
    "services/extensions/internal/modules/payments/subscription_stripe_storage_contract.json": "6452e9ab23c39168e7f90c648970aea4bc5fbfcdddd8e1369a0ed32c072ed4b6",
    "services/extensions/internal/modules/payments/subscription_pancake_schema.sql": "f80fa5b72ff3f4748eadd4787ca7532bead8dc6fa39bcd5b689547c1bcca29a0",
    "services/extensions/internal/modules/payments/subscription_pancake_catalog.sql": "a814b4b97f21c75c6b4b59719177a96aa1ff375591cee286c0137ee1d41715e1",
    "services/extensions/internal/modules/payments/subscription_pancake_storage_contract.json": "2f670d4b1570800652c1e13b44c5981b34c144388f6c73f056a8c108d2613916",
    "services/extensions/internal/modules/checkins/schema.sql": "a5e41379df204162d2aa79fb14fa78102624144b857662be20aa18e87e3ff323",
    "services/extensions/internal/modules/checkins/types.go": "0cc2333c89ff2ccf3ba16c96375a87edf6cc52ad633cb72561b9e129ed03b76a"
}

# Source registration only: Io.net has module PG evidence, not native CLI
# acceptance. ACK-loss, provider/runtime and Core funds remain unqualified.
PENDING_OWNER_BOUNDARIES = []

# Exact SQL predicates and transaction ownership; tests remove each predicate
# with body seals disabled so hash matching cannot conceal a semantic regression.
OWNER_OPERATIONS_GUARDS = (
    "!upgradeOperation.MatchString(operation)",
    "current_user=pg_get_userbyid(d.datdba)",
    "NOT rolcanlogin AND NOT(rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls)",
    "pid<>pg_backend_pid()",
    "pg_prepared_xacts WHERE database=current_database()",
    "nspname='lmm_operations' AND nspowner<>d.datdba",
    "IN ACCESS EXCLUSIVE MODE",
    "SELECT fingerprint FROM lmm_operations.contract WHERE version=3",
    "len(actual) != 32",
    "version=1 AND operation_id=$1 AND database_oid=(SELECT oid FROM pg_database WHERE datname=current_database()) AND installed_by=session_user",
    "if recoverOnly {",
    "return ErrNotFound",
    "a.grantee<>c.relowner",
    "return SealInstalledSchema(ctx, tx)",
)

# Private qualification artifacts, relative to the private cache; never loaded
# by the portable source suite. Hashes bind the reviewed report, not a PG rerun.
OWNER_EVIDENCE = {
    "services/extensions/internal/freshinstall/payments_upgrade.go": ('lmm-completion-20261113/pancake-independent-qualification-1794641819/RECEIPT.json',
        "4e7c62fc69aacac4743722df4131943d9a52a1effeade0cb2c3bbe17aec8fd16"),
    "services/extensions/internal/freshinstall/checkin_policy.go": ('lmm-completion-20261113/owner-cli-private-probes-20261114/ACTUAL_HANDOFF.md',
        "edb40d4a0f6464ce9bc2f6a31cfd10f12640443b6908825c71a2976608a0dd56"),
    "services/extensions/internal/modules/checkins/owner.go": ('lmm-completion-20261113/owner-cli-private-probes-20261114/ACTUAL_HANDOFF.md',
        "edb40d4a0f6464ce9bc2f6a31cfd10f12640443b6908825c71a2976608a0dd56"),
    "services/extensions/internal/modules/payments/subscription_epay_upgrade.go": ('lmm-owner-pg-selftest-20261114-145129/cases/epay/RESULT.json',
        "1a06b3f7b466ed69f5013d4612a894a5fb78a1d7241ea50f5d5d79fd4a826877"),
    "services/extensions/internal/modules/payments/subscription_stripe_upgrade.go": ('lmm-completion-20261113/owner-cli-private-probes-20261114/ACTUAL_HANDOFF.md',
        "edb40d4a0f6464ce9bc2f6a31cfd10f12640443b6908825c71a2976608a0dd56"),
    "services/extensions/internal/modules/payments/subscription_pancake_upgrade.go": ('lmm-completion-20261113/pancake-independent-qualification-1794641819/RECEIPT.json',
        "4e7c62fc69aacac4743722df4131943d9a52a1effeade0cb2c3bbe17aec8fd16"),
    "services/extensions/internal/modules/operations/script_repository_credentials_upgrade.go": ('lmm-owner-pg-selftest-20261114-145129/cases/script/RESULT.json',
        "82a6a377b3063d2ded4c2185eb15e90b9884effa128bc5c1a8e1d80c6dc316c9"),
    "services/extensions/internal/modules/operations/ionet_upgrade.go": ('lmm-owner-pg-selftest-20261114-145129/cases/ionet/RESULT.json',
        "8b60bd207c24e9fcfedb7d06547ce91ff4ac444c63bd247c60e26a5ec2ab5b7e"),
}

OWNER_ENTRY_CALLERS = {
    "RunPaymentsSubscriptionUpgradeCLI": [
        "services/extensions/cmd/extensions/main.go",
        "services/extensions/internal/freshinstall/payments_upgrade.go"
    ],
    "RunPaymentsStripeSubscriptionUpgradeCLI": [
        "services/extensions/cmd/extensions/main.go",
        "services/extensions/internal/freshinstall/payments_upgrade.go"
    ],
    "RunPaymentsPancakeSubscriptionUpgradeCLI": [
        "services/extensions/cmd/extensions/main.go",
        "services/extensions/internal/freshinstall/payments_upgrade.go"
    ],
    "runPaymentsSubscriptionUpgradeCLI": [
        "services/extensions/internal/freshinstall/payments_upgrade.go"
    ],
    "UpgradeSubscriptionEPay": [
        "services/extensions/internal/freshinstall/payments_upgrade.go",
        "services/extensions/internal/modules/payments/subscription_epay_upgrade.go"
    ],
    "UpgradeSubscriptionStripe": [
        "services/extensions/internal/freshinstall/payments_upgrade.go",
        "services/extensions/internal/modules/payments/subscription_stripe_upgrade.go"
    ],
    "UpgradeSubscriptionPancake": [
        "services/extensions/internal/freshinstall/payments_upgrade.go",
        "services/extensions/internal/modules/payments/subscription_pancake_upgrade.go"
    ],
    "RunCheckinPolicyCLI": [
        "services/extensions/cmd/extensions/main.go",
        "services/extensions/internal/freshinstall/checkin_policy.go"
    ],
    "PublishPolicy": [
        "services/extensions/internal/freshinstall/checkin_policy.go",
        "services/extensions/internal/modules/checkins/owner.go"
    ],
    "UpgradeScriptRepositoryCredentials": [
        "services/extensions/internal/freshinstall/operations_upgrade.go",
        "services/extensions/internal/modules/operations/script_repository_credentials_upgrade.go"
    ],
    "UpgradeIoNet": [
        "services/extensions/internal/freshinstall/operations_upgrade.go",
        "services/extensions/internal/modules/operations/ionet_upgrade.go"
    ],
    "RunIoNetUpgradeCLI": [
        "services/extensions/cmd/extensions/main.go",
        "services/extensions/internal/freshinstall/operations_upgrade.go"
    ],
    "RunScriptRepositoryCredentialsUpgradeCLI": [
        "services/extensions/cmd/extensions/main.go",
        "services/extensions/internal/freshinstall/operations_upgrade.go"
    ],
    "runOperationsUpgradeCLI": [
        "services/extensions/internal/freshinstall/operations_upgrade.go"
    ],
    "\"publish-checkin-policy\"": [
        "services/extensions/cmd/extensions/main.go",
        "services/extensions/internal/freshinstall/checkin_policy.go"
    ],
    "\"upgrade-script-repository-credentials\"": [
        "services/extensions/cmd/extensions/main.go",
        "services/extensions/internal/freshinstall/operations_upgrade.go"
    ],
    "\"upgrade-ionet\"": [
        "services/extensions/cmd/extensions/main.go",
        "services/extensions/internal/freshinstall/operations_upgrade.go"
    ],
    "\"upgrade-payments-subscriptions-db\"": [
        "services/extensions/cmd/extensions/main.go",
        "services/extensions/internal/freshinstall/payments_upgrade.go"
    ],
    "\"upgrade-payments-stripe-subscriptions-db\"": [
        "services/extensions/cmd/extensions/main.go",
        "services/extensions/internal/freshinstall/payments_upgrade.go"
    ],
    "\"upgrade-payments-pancake-subscriptions-db\"": [
        "services/extensions/cmd/extensions/main.go",
        "services/extensions/internal/freshinstall/payments_upgrade.go"
    ]
}

CORE_SCHEMAS = (
    "core_abuse.",
    "core_account_appeals.",
    "core_account_lifecycle.",
    "core_account_links.",
    "core_channels.",
    "core_checkins.",
    "core_commerce_external.",
    "core_federated_auth.",
    "core_directory_ads.",
    "core_notifications.",
    "core_operations.",
    "core_passkeys.",
    "core_promotions.",
    "core_referrals.",
    "core_request_limits.",
    "core_subscriptions.",
    "core_assistant.",
    "core_assistant_tools.",
    "core_bounties.",
    "core_gift.",
    "core_humangrant.",
    "core_image.",
    "core_market_oauth.",
    "core_tool_market.",
    "core_wallet.",
    "core_billing.",
    "core_charging.",
    "core_commerce.",
    "core_console.",
    "core_events.",
    "core_history.",
    "core_identity.",
    "core_meta.",
    "core_migration.",
    "core_model.",
    "core_oauth.",
    "core_payments.",
    "core_runtime.",
    "core_schema.",
    "core_telegram_auth.",
    "core_trust_pricing.",
)
# Source guards are an offline contract only. The PostgreSQL role/schema/grant
# mutations in composition/guards_integration_test.go require a real test DB.
COMPOSITION_STORAGE_GUARDS = (
    'regexp.MustCompile(`^lmm_(store|support|assistant|payments|operations|toolmarket|community|acquisition|checkins)(_[a-z0-9]+)*$`)',
    '(c.Component != "store" && c.Component != "support" && c.Component != "assistant" && c.Component != "payments" && c.Component != "operations" && c.Component != "toolmarket" && c.Component != "community" && c.Component != "acquisition" && c.Component != "checkins")',
    '!databaseName.MatchString(c.Database)',
    'strings.HasPrefix(c.Database, "lmm_"+c.Component)',
    'if err := c.validate(); err != nil',
    'coreclient.ReadPrivateFile(c.URLFile, 8192)',
    'name != c.Database',
    'key != "sslmode" && key != "sslrootcert" && key != "connect_timeout" && key != "application_name"',
    'len(values) != 1',
    'sql.Open("postgres", dsn)',
    'db.SetMaxOpenConns(2)',
    'db.SetMaxIdleConns(2)',
    'context.WithTimeout(ctx, 3*time.Second)',
    'rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication',
    r"nspname LIKE 'core\_%' ESCAPE '\'",
    r"nspname LIKE 'lmm\_%' ESCAPE '\' OR nspname='extension_wallet'",
    r"($3 AND nspname NOT LIKE 'pg\_%' ESCAPE '\' AND nspname NOT IN ('public','information_schema'))",
    'NOT(nspname=ANY($1::text[]))',
    'pq.Array(policy.namespaces), c.Database, c.Component == "toolmarket"',
    '{policy.tables, "has_table_privilege", true}',
    '{limits.forbidden, "has_table_privilege", false}',
    '{limits.forbiddenColumns, "has_any_column_privilege", false}',
    'COALESCE(` + check.function + `(current_user,to_regclass(t.name),t.privilege),false)<>$3',
    'err != nil || !valid',
    'limits, _ = storagePolicyFor("assistant", assistantAgentBundle)',
    'if err := checkAssistantAuthority(bounded, db); err != nil',
    'if extra || (required && !slices.Equal(actual, want))',
    'err != nil || excessive || (!allowed && slices.Contains(policy.sequences, seq))',
    'for _, table := range policy.tables',
    "datname LIKE '%core%'",
    r"datname LIKE 'lmm\_%' ESCAPE '\' AND datname <> $2",
    "has_database_privilege(current_user,oid,'CONNECT')",
    "pg_has_role(current_user,oid,'SET')",
    "pg_has_role(current_user,oid,'USAGE')",
    "pg_has_role(current_user,oid,'MEMBER')",
    'if name != c.Database || privileged || foreignSchema || coreConnect || rolePaths',
    'return nil, errors.New("extension storage guard unavailable")',
    'return nil, errors.New("extension storage isolation rejected")',
    'case "operations":',
    'add("lmm_operations.contract", "SELECT")',
    'add("lmm_operations.publications", "SELECT", "INSERT")',
    'add("lmm_operations.reads", "SELECT", "INSERT")',
    'add("lmm_operations.rss_config", "SELECT", "UPDATE")',
    'add("lmm_operations.script_repository", "SELECT", "UPDATE")',
    "has_sequence_privilege(current_user,to_regclass('lmm_operations.publication_id'),$1)",
    'return nil, errors.New("operations sequence grants unavailable")',
    'if failed {',
    '_ = db.Close()',
)
COMPOSITION_RUNTIME_GUARDS = (
    'openStorage(ctx, cfg.Checkins)',
    'checkins.New(ctx, db, coreclient.CheckinLedger{Client: client})',
    'openStorage(ctx, cfg.Store)',
    'openStorage(ctx, cfg.Support)',
    'openStorageBundle(ctx, cfg.Assistant, assistantStorageBundle(cfg))',
    'openStorage(ctx, cfg.Operations)',
    'openStorage(ctx, cfg.Community)',
    'openStorage(ctx, cfg.Acquisition)',
    'store.NewPostgres(db)',
    'support.NewPostgres(db)',
    'community.New(ctx, db, cfg.HostPublicOrigin, community.Options{Core: reader, AdCore: adCore})',
    'acquisition.New(ctx, db, cfg.HostPublicOrigin, !cfg.LocalHTTP)',
    'var funds store.Funds = store.DisabledFunds{}',
    'if cfg.CommerceTokenFile != ""',
    'if err := independentRuntimeCredentials(cfg.HostCredential, token, commerceToken); err != nil',
    'rustfunds.New(r.CommerceClient, caps.GetFeatures())',
    'store.New(repo, auth, funds)',
)

# This remains an offline source contract, not an execution of Go. Tokenize
# literals/comments so a commented-out guard cannot satisfy a call/body check.
# Require the complete small pairwise helper, not an error-message substring.
GO_TOKEN = re.compile(
    r'//[^\n]*|/\*[\s\S]*?\*/|"(?:\\.|[^"\\])*"|`[^`]*`|'
    r"'(?:\\.|[^'\\])*'|[A-Za-z_]\w*|\d+|\.\.\.|:=|==|!=|&&|\|\||[^\s]"
)
RUNTIME_CREDENTIAL_BODY = """
for i, token := range credentials {
    if len(token) == 0 {
        continue
    }
    for _, other := range credentials[i+1:] {
        if len(other) != 0 && bytes.Equal(token, other) {
            return errors.New("host and core RPC credentials must be independent")
        }
    }
}
return nil
"""
PAYMENT_CREDENTIAL_BODY = """
paymentToken, err := coreclient.ReadTokenFile(cfg.PaymentTokenFile)
if err != nil {
    return nil, errors.New("private payment RPC credential unavailable")
}
if len(cfg.HostCredential) < 32 {
    return nil, errors.New("payment RPC requires an independent loaded host credential")
}
if err := independentRuntimeCredentials(cfg.HostCredential, identityToken, commerceToken, paymentToken); err != nil {
    return nil, err
}
return paymentToken, nil
"""
ASSISTANT_CREDENTIAL_BODY = """
if cfg.AssistantTokenFile == "" {
    return nil, nil
}
if len(cfg.HostCredential) < 32 {
    return nil, errors.New("assistant RPC requires an independent loaded host credential")
}
token, err := coreclient.ReadTokenFile(cfg.AssistantTokenFile)
if err != nil {
    return nil, errors.New("private assistant RPC credential unavailable")
}
if err = independentRuntimeCredentials(cfg.HostCredential, identity, commerce, payment, token); err != nil {
    return nil, err
}
return token, nil
"""
OPERATIONS_RUNTIME_BLOCK = """
if selected["operations"] {
    config := operations.Config{Scripts: cfg.Scripts}
    if cfg.Operations.URLFile != "" || cfg.Operations.Database != "" {
        db, err := openStorage(ctx, cfg.Operations)
        if err != nil { return nil, err }
        r.close = append(r.close, db.Close)
        config.Releases = db
    }
    if config.Releases == nil && config.Scripts == nil {
        return nil, errors.New("operations requires configured releases or reviewed scripts")
    }
    r.Operations, err = operations.New(ctx, config)
    if err != nil { return nil, err }
    if cfg.ScriptRepositoryKeyFile != "" {
        key, err := scriptRepositoryKey(cfg, cfg.HostCredential, token, commerceToken, paymentToken, assistantToken, supportCustomerToken, supportNoteKey)
        if err != nil { return nil, err }
        if err := r.Operations.Repository.WithCredentialKey(ctx, key); err != nil { return nil, err }
    }
    if cfg.IoNetKeyFile != "" {
        key, err := ioNetKey(cfg, cfg.HostCredential, token, commerceToken, paymentToken, assistantToken, supportCustomerToken, supportNoteKey)
        if err != nil { return nil, err }
        if err := r.Operations.IoNet.WithCredentialKey(ctx, key); err != nil { return nil, err }
    }
} else if cfg.Operations.URLFile != "" || cfg.Operations.Database != "" || cfg.Scripts != nil || cfg.ScriptRepositoryKeyFile != "" || cfg.IoNetKeyFile != "" {
    return nil, errors.New("operations configuration requires explicit module selection")
}
"""
OPERATIONS_NEW_BODY = """
s := new(Service)
var err error
if c.Releases != nil {
    s.Releases, err = NewPostgres(ctx, c.Releases)
    if err != nil { return nil, err }
    s.RSS = newRSS(c.Releases)
    s.Repository = NewScriptRepository(c.Releases)
    var ionet bool
    if c.Releases.QueryRowContext(ctx, `SELECT to_regclass('lmm_operations.ionet_config') IS NOT NULL`).Scan(&ionet) != nil {
        return nil, ErrUnavailable
    }
    if ionet { s.IoNet = NewIoNet(c.Releases) }
}
if c.Scripts != nil {
    s.Scripts, err = LoadScripts(*c.Scripts)
    if err != nil { return nil, err }
}
return s, nil
"""
OPERATIONS_LIST_BODY = """
if limit < 1 || limit > 50 { return nil, ErrInvalid }
ctx, cancel := context.WithTimeout(ctx, 3*time.Second)
defer cancel()
rows, err := s.db.QueryContext(ctx, "SELECT "+noteColumns+" FROM lmm_operations.publications ORDER BY id DESC LIMIT $1", limit)
if err != nil { return nil, ErrUnavailable }
defer rows.Close()
out := []ReleaseNote{}
for rows.Next() {
    n, e := scanNote(rows)
    if e != nil { return nil, ErrUnavailable }
    out = append(out, *n)
}
if rows.Err() != nil { return nil, ErrUnavailable }
return out, nil
"""
OPERATIONS_SEQUENCE_BLOCK = """
if c.Component == "operations" {
    for _, privilege := range []string{"USAGE", "SELECT"} {
        var allowed bool
        if err := db.QueryRowContext(bounded, `SELECT COALESCE(has_sequence_privilege(current_user,to_regclass('lmm_operations.publication_id'),$1),false)`, privilege).Scan(&allowed); err != nil || !allowed {
            return nil, errors.New("operations sequence grants unavailable")
        }
    }
}
"""
OPERATIONS_CHECK_GUARDS = (
    "rolsuper OR rolcreaterole OR rolcreatedb OR rolreplication OR rolbypassrls",
    "FROM pg_auth_members", "has_database_privilege(current_database(),'CREATE')",
    "has_database_privilege(current_database(),'TEMP')", "has_schema_privilege(oid,'CREATE')",
    "c.relrowsecurity OR c.relforcerowsecurity", "FROM pg_policy",
    "has_table_privilege($1,'UPDATE')=$3", "'DELETE,TRUNCATE,REFERENCES,TRIGGER'",
    "a.is_grantable OR a.privilege_type NOT IN", "WHEN c.relname IN ('rss_config','script_repository','ionet_config') THEN 'UPDATE'",
    "aclexplode(a.attacl)", "has_sequence_privilege('lmm_operations.publication_id','USAGE')",
    "has_sequence_privilege('lmm_operations.publication_id','SELECT')",
    "NOT has_sequence_privilege('lmm_operations.publication_id','UPDATE')",
)


def go_tokens(source):
    return [token for token in GO_TOKEN.findall(source)
            if not token.startswith(("//", "/*"))]


class CoreBoundaryTests(unittest.TestCase):
    def test_services_and_user_apps_have_separate_roots(self):
        for name in ("apps/lmm-core", "apps/lmm-extensions", "apps/lmm"):
            self.assertFalse((ROOT / name).exists(), name)
        for name in ("services/core/Cargo.toml", "services/extensions/go.mod",
                     "apps/web/package.json"):
            self.assertTrue((ROOT / name).is_file(), name)

    def test_mise_pins_match_component_toolchain_contracts(self):
        tools = tomllib.loads((ROOT / "mise.toml").read_text())["tools"]
        rust = tomllib.loads((ROOT / "services/core/rust-toolchain.toml").read_text())["toolchain"]
        package = json.loads((ROOT / "package.json").read_text())
        self.assertEqual(tools["rust"]["version"], rust["channel"])
        self.assertEqual(tools["rust"]["profile"], rust["profile"])
        self.assertEqual(set(tools["rust"]["components"]), set(rust["components"]))
        self.assertEqual(f"bun@{tools['bun']}", package["packageManager"])

    def test_retired_backends_and_migration_entry_points_are_absent(self):
        for name in ("apps/api-rust", "apps/extensions-go", "apps/core-rust", "apps/api-go", "services/core/migrations",
                     "services/extensions/model", "services/extensions/controller", "services/extensions/relay",
                     "services/extensions/router", "services/extensions/migration", "services/extensions/internal/appcli",
                     "scripts/ci/qualify-go-migration-startup.sh", "scripts/deploy-systemd.py",
                     "scripts/deploy-shared-postgres.py", "packaging/common/lmm-api/migration-compatibility.env"):
            self.assertFalse((ROOT / name).exists(), name)
        self.assertTrue((ROOT / "services/extensions/cmd/extensions/main.go").is_file())

    def test_cli_is_an_independent_pinned_subproject(self):
        entry = subprocess.run(['git', 'ls-files', '--stage', '--', 'apps/cli'],
                               cwd=ROOT, check=True, capture_output=True, text=True).stdout.strip()
        self.assertRegex(entry, r'^160000 [0-9a-f]{40} 0\tapps/cli$')
        modules = configparser.ConfigParser(interpolation=None)
        modules.read(ROOT / '.gitmodules')
        self.assertEqual(modules['submodule "apps/cli"']['path'], 'apps/cli')
        self.assertEqual(modules['submodule "apps/cli"']['url'],
                         'https://github.com/TokenNotIncluded/lmm.git')

    def test_extension_has_no_core_database_or_authority_dependency(self):
        module = (ROOT / "services/extensions/go.mod").read_text()
        for forbidden in ("gorm.io", "go-sql-driver", "jackc/pgx", "sqlite"):
            self.assertNotIn(forbidden, module)
        self.assertRegex(module, r"(?m)^\s*github\.com/lib/pq v[0-9.]+\s*$")
        for path in (ROOT / "services/extensions").rglob("*.go"):
            if path.name.endswith("_test.go"):
                continue
            text = path.read_text()
            self.assertNotIn("AutoMigrate(", text, str(path))
            self.assert_extension_storage(path.relative_to(ROOT).as_posix(), text)
            if path.name != "environment.go":
                for forbidden in ("SQL_DSN", "LMM_CORE_DATABASE_URL"):
                    self.assertNotIn(forbidden, text, str(path))
        startup = (ROOT / "services/extensions/internal/app/run.go").read_text()
        self.assertLess(startup.index("validateEnvironment(os.Getenv)"), startup.index("LoadConfig(path)"))
        self.assertLess(startup.index("LoadConfig(path)"), startup.index("readHostCredential(cfg.Host.TokenFile)"))
        identity = (ROOT / "services/extensions/internal/modules/identity/identity.go").read_text()
        self.assertIn("type Core interface", identity)
        for operation in ("Capabilities", "Authorize", "ListTeams"):
            self.assertIn(operation, identity)
        self.assertIn("corepb", identity)

    def assert_borrowed_pool_not_closed(self, path, text):
        if path == COMPOSITION_STORAGE:
            return  # The sole pool owner closes it on startup failure/shutdown.
        tokens = go_tokens(text)
        if path in {
            "services/extensions/internal/composition/runtime.go",
            "services/extensions/internal/composition/payments.go",
            "services/extensions/internal/composition/toolmarket.go",
        }:
            owned_close = go_tokens("r.close = append(r.close, db.Close)")
            opens = sum(tokens[i:i+2] == ["openStorage", "("] or tokens[i:i+2] == ["openStorageBundle", "("] for i in range(len(tokens)-1))
            self.assertGreater(opens, 0, path)
            self.assertEqual(sum(tokens[i:i+len(owned_close)] == owned_close for i in range(len(tokens))), opens, path)
            stripped = []
            i = 0
            while i < len(tokens):
                if tokens[i:i+len(owned_close)] == owned_close:
                    i += len(owned_close)
                else:
                    stripped.append(tokens[i]); i += 1
            tokens = stripped
        code = " ".join(t if not t.startswith(('"', "`", "'")) else "LITERAL" for t in tokens)
        # Track common injected DB fields and any explicitly typed DB parameter,
        # including aliases/method values. Rows returned by Query remain closable.
        borrowed = {"db", "pg", "Database", "Releases"}
        borrowed.update(re.findall(r"\b(\w+) \* sql \. DB\b", code))
        for _ in range(len(tokens)):
            aliases = set()
            for name, rhs in re.findall(r"\b(\w+) (?:=|:=) (\w+(?: \. \w+)*)", code):
                parts = rhs.strip().split(" . ")
                if parts[-1] in borrowed:
                    aliases.add(name)
            if aliases <= borrowed:
                break
            borrowed.update(aliases)
        for name in borrowed:
            self.assertNotRegex(code, r"\b" + re.escape(name) + r" \. Close\b", path)

    def assert_reviewed_owner_boundary(self, path, text, *, check_seal=True):
        tokens = go_tokens(text)
        if path.endswith("freshinstall/payments_upgrade.go"):
            for guard in ("db.SetMaxOpenConns(1)", "db.SetMaxIdleConns(1)",
                          "SET ROLE ", "session_user<>$3", "NOT rolcanlogin",
                          "pg_has_role(oid,$2,'SET')", "pg_has_role(oid,$3,'SET')",
                          'if recoverOnly && receipt.Status != "already_committed"'):
                self.assertIn(guard, text, path)
            for provider in ("EPay", "Stripe", "Pancake"):
                self.token_index(tokens, "payments.UpgradeSubscription" + provider + "(ctx, db, c.ExpectedDatabase, c.RuntimeRole)")
        elif "/payments/" in path:
            for guard in ("sql.LevelSerializable", "SET LOCAL synchronous_commit=on",
                          "SET LOCAL search_path=pg_catalog", "pg_advisory_xact_lock",
                          "database != expectedDatabase", "after != before", "afterACL != acl",
                          "tx.Commit()", "receipt.Status = \"already_committed\""):
                self.assertIn(guard, text, path)
            if not path.endswith("subscription_epay_upgrade.go"):
                self.token_index(tokens, "if len(target.After) != 64 { return receipt, ErrUnavailable }")
        elif "/operations/" in path:
            for guard in OWNER_OPERATIONS_GUARDS:
                self.assertIn(guard, text, path)
            if path.endswith("ionet_upgrade.go"):
                self.assertIn("NOT has_database_privilege($1,d.oid,'CREATE,TEMPORARY')", text, path)
                self.assertIn('"ionet_config", "ionet_operations", "ionet_upgrades"', text, path)
                self.assertIn("GRANT SELECT,UPDATE ON lmm_operations.ionet_config TO ", text, path)
                self.assertIn("GRANT SELECT,INSERT,UPDATE ON lmm_operations.ionet_operations TO ", text, path)
            else:
                self.assertIn("LOCK TABLE lmm_operations.contract,lmm_operations.script_repository IN ACCESS EXCLUSIVE MODE", text, path)
                self.assertIn("c.oid='lmm_operations.script_repository_credentials_upgrades'::regclass", text, path)
        elif path.endswith("freshinstall/checkin_policy.go"):
            for guard in ("d.DisallowUnknownFields()", "current_user<>$3",
                          "pg_get_userbyid(datdba)=$2", "SET ROLE ", "pq.QuoteIdentifier(c.OwnerRole)",
                          "version != 1", "command.Policy.Validate() != nil"):
                self.assertIn(guard, text, path)
            self.token_index(tokens, "checkins.PublishPolicy(ctx, db, command.OperationKey, command.ExpectedRevision, command.Policy)")
        else:
            for guard in ("sql.LevelSerializable", "SET LOCAL synchronous_commit=on",
                          "pg_advisory_xact_lock", "revision != expected+1",
                          "current != expected", "revision = current + 1", "tx.Commit()"):
                self.assertIn(guard, text, path)
            self.token_index(tokens, "if !matches || revision != expected+1 { return 0, ErrConflict }")
            self.token_index(tokens, "if current != expected { return 0, ErrConflict }")
        if check_seal:
            actual = hashlib.sha256(json.dumps(tokens, separators=(",", ":")).encode()).hexdigest()
            self.assertEqual(actual, REVIEWED_OWNER_BOUNDARIES[path], path + ": owner closure changed")
            for source, expected in REVIEWED_OWNER_INPUTS.items():
                self.assertEqual(hashlib.sha256((ROOT / source).read_bytes()).hexdigest(), expected,
                                 source + ": embedded owner contract changed; new qualification required")

    def assert_offline_installation_boundary(self, path, text):
        tokens = go_tokens(text)
        for symbol, callers in OWNER_ENTRY_CALLERS.items():
            if path not in callers:
                self.assertNotIn(symbol, tokens, path + ": offline owner call leaked")
        if path in REVIEWED_OWNER_BOUNDARIES:
            self.assert_reviewed_owner_boundary(path, text)
            return True
        if path in PENDING_OWNER_BOUNDARIES:
            self.fail(path + ": owner install/ACL/recovery evidence pending; not runtime admission")
        imported = "github.com/TokenNotIncluded/lmm-api/extensions/internal/freshinstall"
        if path not in (OFFLINE_MAIN, OFFLINE_INSTALLER, OFFLINE_CLI, OFFLINE_WALLET_CLI, OFFLINE_WALLET_UPGRADE, OFFLINE_OPERATIONS_CLI):
            self.assertNotIn(imported, text, path)
            self.assertNotRegex(" ".join(go_tokens(text)), r"\bfreshinstall \.", path)
            for owner_entry in ('"install-db"', '"owner-url-file"', 'OwnerURLFile'):
                self.assertNotIn(owner_entry, go_tokens(text), path)
        wallet_import = "github.com/TokenNotIncluded/lmm-api/extensions/internal/walletupgrade"
        if path not in (OFFLINE_MAIN, OFFLINE_WALLET_CLI, OFFLINE_WALLET_UPGRADE):
            self.assertNotIn(wallet_import, text, path)
            self.assertNotRegex(" ".join(go_tokens(text)), r"\bwalletupgrade \.", path)
            self.assertNotIn('"upgrade-wallet-db"', go_tokens(text), path)
        if path not in (OFFLINE_SNAPSHOT, OFFLINE_INSTALLER):
            self.assertNotIn("InitPostgresSnapshot", text, path)
        # RSS installation is a separate, frozen owner entry point. None of
        # these calls may be borrowed by normal startup or request handlers.
        owner_calls = {
            "compiledCommerce": (OFFLINE_INSTALLER, OFFLINE_COMMERCE_PLAN),
            "RunOperationsUpgradeCLI": (OFFLINE_MAIN, OFFLINE_OPERATIONS_CLI),
            "UpgradeRSS": (OFFLINE_OPERATIONS_CLI, OFFLINE_RSS_UPGRADE),
            "UpgradeScriptRepository": (OFFLINE_OPERATIONS_CLI, OFFLINE_REPOSITORY_UPGRADE),
            "SealInstalledSchema": (OFFLINE_INSTALLER, OFFLINE_RSS_UPGRADE, OFFLINE_REPOSITORY_UPGRADE, OPERATIONS_POSTGRES),
            "sealInstalledSchemaVersion": (OPERATIONS_POSTGRES, OFFLINE_RSS_UPGRADE),
            '"upgrade-operations-db"': (OFFLINE_MAIN, OFFLINE_OPERATIONS_CLI),
        }
        for name, allowed in owner_calls.items():
            if path not in allowed:
                self.assertNotIn(name, go_tokens(text), path)
        if path == OFFLINE_REPOSITORY_UPGRADE:
            self.assertIn(path, OFFLINE_TOKEN_SHA256,
                          "APP01 owner upgrade requires an independently reviewed source fingerprint")
        if path == OFFLINE_OPERATIONS_CLI:
            self.assert_operations_upgrade_dispatch(text)
        if path in OFFLINE_TOKEN_SHA256:
            actual = hashlib.sha256(json.dumps(go_tokens(text), separators=(",", ":")).encode()).hexdigest()
            self.assertEqual(actual, OFFLINE_TOKEN_SHA256[path], path + ": offline owner body needs independent review")
            if path in (OFFLINE_MAIN, OFFLINE_INSTALLER, OFFLINE_COMMERCE_PLAN, OFFLINE_CLI, OFFLINE_WALLET_CLI, OFFLINE_WALLET_UPGRADE, OFFLINE_OPERATIONS_CLI, OFFLINE_RSS_UPGRADE, OFFLINE_REPOSITORY_UPGRADE):
                return True
        return False

    def assert_extension_storage(self, path, text):
        offline = self.assert_offline_installation_boundary(path, text)
        tokens = go_tokens(text)
        if path not in (COMPOSITION_STORAGE, "services/extensions/internal/composition/toolmarket.go", "services/extensions/internal/composition/runtime.go"):
            self.assertNotIn("openStorageBundle", tokens, path)
        if path not in (COMPOSITION_STORAGE, COMPOSITION_BUNDLE, "services/extensions/internal/composition/toolmarket.go"):
            self.assertNotIn("toolMarketBuiltinsBundle", tokens, path)
        if path == "services/extensions/internal/composition/runtime.go":
            body = self.go_function_body(text, "func New(ctx context.Context, cfg Config) (_ *Runtime, resultErr error)")
            self.token_index(body, """if selected["assistant"] {
                db, err := openStorageBundle(ctx, cfg.Assistant, assistantStorageBundle(cfg))
                if err != nil { return nil, err }
                r.close = append(r.close, db.Close)
            """)
            self.assertEqual(tokens.count("openStorageBundle"), 1, path)
        if path == COMPOSITION_BUNDLE:
            actual = hashlib.sha256(json.dumps(tokens, separators=(",", ":")).encode()).hexdigest()
            self.assertEqual(actual, BUNDLE_POLICY_TOKEN_SHA256, "complete storage bundle policy requires review")
        if path == "services/extensions/internal/composition/toolmarket.go":
            body = self.go_function_body(text, "func (r *Runtime) composeToolMarket(ctx context.Context, cfg Config, otherCredentials ...[]byte) error")
            self.token_index(body, """
                bundle := defaultStorageBundle
                if cfg.ToolMarketBuiltins.configured() {
                    bundle = toolMarketBuiltinsBundle
                }
                db, err := openStorageBundle(ctx, cfg.ToolMarket, bundle)
            """)
            self.assertEqual(tokens.count("openStorageBundle"), 1, path)
            self.assertEqual(tokens.count("toolMarketBuiltinsBundle"), 1, path)
        # Borrowed pools can execute SQL from files without a database/sql import.
        # Check Core table access before any unregistered-file early return.
        # Include quoted/future Core schemas: a new registry entry cannot open
        # a hole before this explicit catalogue is maintained.
        self.assertNotRegex(text, r'(?i)\b(?:FROM|JOIN|UPDATE|INTO|TRUNCATE(?:\s+TABLE)?|REFERENCES|(?:ALTER|CREATE|DROP)\s+TABLE)\s+"?core_[a-z0-9_]+"?\s*\.', path)
        if offline:
            return  # Exact reviewed owner/CLI bodies; never a runtime exception.
        self.assert_borrowed_pool_not_closed(path, text)
        if path in REVIEWED_BORROWED_SQL:
            self.assert_registered_borrowed_sql(path, text)
            return
        if path in STORE_READ_QUERIES:
            method, query = STORE_READ_QUERIES[path]
            # Only the reviewed single literal SELECT is allowed in these adapters.
            calls = re.findall(r"\.\s*(Query|QueryRow|Exec|QueryContext|QueryRowContext|ExecContext|Begin|BeginTx)\s*\(", text)
            self.assertEqual(calls, [method], path)
            self.token_index(tokens, "t.tx." + method + "(t.ctx, " + query + ",")
        stores = {
            "services/extensions/internal/coreclient/sqlinbox.go": "extension_events.",
            "services/extensions/internal/modules/store/postgres.go": "lmm_store.",
            "services/extensions/internal/modules/store/promotions.go": "lmm_store.",
            "services/extensions/internal/modules/store/purchase_limits.go": "lmm_store.",
            "services/extensions/internal/modules/store/postgres_web.go": "lmm_store.",
            "services/extensions/internal/modules/store/postgres_collections.go": "lmm_store.",
            "services/extensions/internal/modules/store/money_recovery_postgres.go": "lmm_store.",
            "services/extensions/internal/modules/support/postgres.go": "lmm_support.",
            "services/extensions/internal/modules/support/postgres_web.go": "lmm_support.",
            "services/extensions/internal/modules/assistant/postgres.go": "lmm_assistant.",
            "services/extensions/internal/modules/assistant/history_postgres.go": "lmm_assistant.",
            "services/extensions/internal/modules/assistant/delegation_pending_postgres.go": "lmm_assistant.",
            "services/extensions/internal/modules/assistant/delegation_postgres.go": "lmm_assistant.",
            "services/extensions/internal/modules/promotions/postgres.go": "lmm_promotions.",
            "services/extensions/internal/modules/toolmarket/postgres.go": "lmm_toolmarket.",
            OFFLINE_SNAPSHOT: "lmm_toolmarket.",
            "services/extensions/internal/modules/wallet/postgres.go": "extension_wallet.",
            "services/extensions/internal/modules/bounties/postgres.go": "lmm_bounties.",
            "services/extensions/internal/modules/assistant/agent_postgres.go": "lmm_assistant.",
            "services/extensions/internal/modules/assistant/personalization/service.go": "lmm_assistant.",
            "services/extensions/internal/modules/assistant_services/service.go": "lmm_assistant.",
            "services/extensions/internal/modules/assistant_services/memory.go": "lmm_assistant.",
            "services/extensions/internal/modules/assistant_services/confirmation.go": "lmm_assistant.",
            "services/extensions/internal/modules/assistant_services/issues.go": "lmm_assistant.",
            "services/extensions/internal/modules/assistant_services/platform_support.go": "lmm_assistant.",
            "services/extensions/internal/modules/assistant_services/search.go": "lmm_assistant.",
            "services/extensions/internal/modules/assistant_services/support_messages.go": "lmm_assistant.",
            "services/extensions/internal/modules/assistant_services/web_support.go": "lmm_assistant.",
            "services/extensions/internal/modules/payments/postgres.go": "payment_",
            OPERATIONS_POSTGRES: "lmm_operations.",
            OPERATIONS_RELEASES: "lmm_operations.",
            OPERATIONS_RSS: "lmm_operations.",
            OPERATIONS_REPOSITORY: "lmm_operations.",
        }
        stores.update({f"services/extensions/internal/modules/{module}/{name}": f"lmm_{module}."
                       for module, names in MODULE_STORAGE_FILES.items() for name in names})
        # Exactly one runtime file opens bounded pools. The operations owner
        # initializer uses pq for identifier quoting; three registered adapters
        # use only Array serialization. All adapters borrow their injected pool.
        for forbidden in ("gorm.io", "go-sql-driver", "jackc/pgx", "sqlite"):
            self.assertNotIn(forbidden, text, path)
        if path != COMPOSITION_STORAGE:
            if path == OPERATIONS_POSTGRES:
                self.assert_operations_postgres(text)
            elif path in PQ_ARRAY_ADAPTERS:
                self.assert_named_go_import(text, "github.com/lib/pq")
                selectors = [tokens[i+2] for i in range(len(tokens)-2) if tokens[i:i+2] == ["pq", "."]]
                self.assertEqual(selectors, ["Array"], path)
            else:
                self.assertNotIn("github.com/lib/pq", text, path)
                if path == OPERATIONS_RELEASES:
                    self.assert_operations_releases(text)
            self.assertNotRegex(text, r"\bsql\s*\.\s*(?:Open|OpenDB)\b", path)
        if path == OPERATIONS_INJECTION:
            self.assert_operations_injection(text)
            # The exact constructor above admits one catalog-only probe; it
            # does not grant arbitrary operations SQL to the composition file.
            catalog_probe = "`SELECT to_regclass('lmm_operations.ionet_config') IS NOT NULL`"
            self.assertEqual(text.count(catalog_probe), 1)
            without_probe = text.replace(catalog_probe, "CATALOG_PROBE", 1)
            for schema in CORE_SCHEMAS + tuple(stores.values()):
                self.assertNotIn(schema, without_probe, path)
            return
        if path == OPERATIONS_RSS:
            self.assertEqual(re.findall(r"\.\s*(Query|QueryRow|Exec|QueryContext|QueryRowContext|ExecContext|Begin|BeginTx)\s*\(", text),
                             ["QueryRowContext", "QueryRowContext"], path)
            for query in (
                "s.db.QueryRowContext(ctx, `SELECT revision,feeds FROM lmm_operations.rss_config WHERE singleton`)",
                "s.db.QueryRowContext(ctx, `UPDATE lmm_operations.rss_config SET feeds=$1,revision=revision+1,updated_by=$2 WHERE singleton AND revision=$3 RETURNING revision`, string(raw), actor, settings.Revision)",
            ):
                self.token_index(tokens, query)
        if path == OPERATIONS_REPOSITORY:
            self.assert_script_repository_storage(text)
        if path == COMPOSITION_ASSISTANT:
            self.assert_assistant_composition(text)
            for schema in CORE_SCHEMAS + tuple(stores.values()):
                self.assertNotIn(schema, text, path)
            return
        type_only = {
            OPERATIONS_REPOSITORY_FETCH,
            "services/extensions/internal/modules/toolmarket/store.go",
            "services/extensions/internal/modules/assistant_services/actions.go",
            "services/extensions/internal/modules/assistant_services/dispatch.go",
        }
        if '"database/sql"' in text:
            tokens = self.assert_named_go_import(text, "database/sql")
            for i in range(len(tokens)-2):
                if tokens[i:i+2] == ["sql", "."] and path != COMPOSITION_STORAGE:
                    self.assertIn(tokens[i+2], {"DB", "Tx", "TxOptions", "Row", "Rows", "Result", "ErrNoRows", "NullString", "NullInt64", "NullTime", "LevelReadCommitted", "LevelRepeatableRead"}, path)
        if path == "services/extensions/internal/modules/assistant/greeting.go":
            advisory = "tx.ExecContext(ctx, `SELECT pg_advisory_xact_lock(hashtextextended($1,0))`, \"lmm_assistant:\"+p.Account.Key())"
            self.assertEqual(text.count(advisory), 2, path)
            self.assertEqual(len(re.findall(r"\.\s*(?:Query|QueryRow|Exec|QueryContext|QueryRowContext|ExecContext|Begin|BeginTx)\s*\(", text)), 2, path)
            for schema in CORE_SCHEMAS + tuple(stores.values()):
                self.assertNotIn(schema, text, path)
            return
        if path in type_only:
            if path == OPERATIONS_REPOSITORY_FETCH:
                selectors = [tokens[i+1] for i in range(len(tokens)-1) if tokens[i] == "."]
                self.assertFalse(SQL_METHODS.intersection(selectors), path)
            else:
                self.assertNotRegex(text, r"\.\s*(?:Query|QueryRow|Exec|QueryContext|QueryRowContext|ExecContext|Begin|BeginTx)\s*\(", path)
            for schema in CORE_SCHEMAS + tuple(stores.values()):
                self.assertNotIn(schema, text, path)
            return
        if path == "services/extensions/internal/composition/builtins.go":
            expected = "SELECT has_schema_privilege(current_user,'extension_wallet','USAGE') AND has_table_privilege(current_user,'extension_wallet.transfer_links','SELECT') AND has_table_privilege(current_user,'extension_wallet.transfer_links','INSERT')"
            self.assertIn("`" + expected + "`", text, path)
            self.assertEqual(len(re.findall(r"\.\s*(?:Query|QueryRow|Exec|QueryContext|QueryRowContext|ExecContext|Begin|BeginTx)\s*\(", text)), 1, path)
            for schema in CORE_SCHEMAS + tuple(stores.values()):
                self.assertNotRegex(text, r"(?i)\b(?:FROM|JOIN|UPDATE|INTO|TRUNCATE)\s+" + re.escape(schema), path)
            return
        if path not in stores and path != COMPOSITION_STORAGE:
            self.assertNotIn('"database/sql"', text, path)
            # Borrowing a pool via another struct cannot bypass registration.
            self.assertNotRegex(text, r'(?i)\b(?:FROM|JOIN|UPDATE|INTO|TRUNCATE)\s+(?:"?(?:lmm_|extension_)[a-z0-9_]+"?\s*\.|(?:public\.)?payment_)', path)
            return
        if path == COMPOSITION_STORAGE:
            actual = hashlib.sha256(json.dumps(tokens, separators=(",", ":")).encode()).hexdigest()
            self.assertEqual(actual, STORAGE_TOKEN_SHA256, "storage authority body requires review")
            self.assertIn('"github.com/lib/pq"', text, path)
            self.assertEqual(len(re.findall(r"\bsql\s*\.\s*(?:Open|OpenDB)\s*\(", text)), 1)
            selectors = [tokens[i+2] for i in range(len(tokens)-2) if tokens[i:i+2] == ["sql", "."]]
            self.assertEqual(selectors.count("Open"), 1, path)
            self.assertNotIn("OpenDB", selectors, path)
            for required in COMPOSITION_STORAGE_GUARDS:
                self.assertIn(required, text, path)
            self.assert_operations_storage(text)
            self.assert_storage_addons(text)
            wrapper = self.go_function_body(text, "func openStorage(ctx context.Context, c StorageConfig) (*sql.DB, error)")
            self.assertEqual(wrapper, go_tokens("return openStorageBundle(ctx, c, defaultStorageBundle)"))
            pq_selectors = [tokens[i+2] for i in range(len(tokens)-2) if tokens[i:i+2] == ["pq", "."]]
            self.assertTrue(pq_selectors and set(pq_selectors) == {"Array"}, path)
        else:
            self.assertIn(stores[path], text, path)
        for referenced in re.findall(r'(?i)\b((?:lmm_|extension_)[a-z0-9_]+)"?\s*\.', text):
            if path != COMPOSITION_STORAGE:
                self.assertEqual(referenced.lower() + ".", stores.get(path), path)
        schemas = CORE_SCHEMAS + tuple(stores.values())
        for schema in schemas:
            if schema != stores.get(path):
                if path == COMPOSITION_STORAGE or schema == "payment_":
                    # Permission introspection names are not business-table
                    # access. The resource boundary must never query those rows.
                    qualifier = r"(?:public\.)?" if schema == "payment_" else ""
                    self.assertNotRegex(text, r"(?i)\b(?:FROM|JOIN|UPDATE|INTO|TRUNCATE)\s+" + qualifier + re.escape(schema), path)
                else:
                    self.assertNotIn(schema, text, path)

    def assert_registered_borrowed_sql(self, path, text, *, check_seal=True):
        spec = REVIEWED_BORROWED_SQL[path]
        tokens = go_tokens(text)
        self.assert_borrowed_pool_not_closed(path, text)
        for forbidden in ("gorm.io", "go-sql-driver", "jackc/pgx", "sqlite"):
            self.assertNotIn(forbidden, text, path)
        self.assertNotRegex(" ".join(tokens), r"\bsql \. (?:Open|OpenDB)\b", path)
        methods = [tokens[i + 2] for i in range(len(tokens) - 2)
                   if tokens[i + 1] == "." and tokens[i + 2] in SQL_METHODS | {"Open", "OpenDB"}]
        self.assertEqual(methods, list(spec["methods"]), path)
        self.assertFalse(set(methods) & {"Conn", "Driver", "Raw", "Open", "OpenDB", "Prepare", "PrepareContext"}, path)
        if '"github.com/lib/pq"' in tokens:
            self.assertTrue(path.endswith("/operations/ionet_journal.go"), path)
            self.assert_named_go_import(text, "github.com/lib/pq")
            selectors = [tokens[i+2] for i in range(len(tokens)-2) if tokens[i:i+2] == ["pq", "."]]
            self.assertEqual(selectors, ["Error"], path)
        if '"database/sql"' in tokens:
            self.assert_named_go_import(text, "database/sql")
        queries = [t[1:-1] for t in tokens if t.startswith(('"', "`"))
                   and re.match(r"^\s*(?:SELECT|INSERT|UPDATE|DELETE|WITH|SET|CREATE|ALTER|DROP|GRANT|REVOKE|LOCK|TRUNCATE)\b", t[1:-1])]
        self.assertTrue(queries, path)
        for query in queries:
            self.assertNotRegex(query, r"(?i)\b(?:CREATE|ALTER|DROP|GRANT|REVOKE|TRUNCATE)\b", path)
            self.assertNotRegex(query, r"(?i)\bcore_[a-z0-9_]+\s*\.", path)
            # EXTRACT(epoch FROM ...) contains SQL syntax, not a relation.
            relations_sql = query.replace("extract(epoch FROM statement_timestamp())", "SQL_CLOCK")
            for relation in re.findall(r"(?i)\b(?:FROM|JOIN|INTO|UPDATE)\s+([a-z_][a-z0-9_.]*)", relations_sql):
                if relation.upper() != "SET":  # ON CONFLICT DO UPDATE SET ...
                    self.assertIn(relation, spec["tables"], path)
        for predicate in spec["predicates"]:
            self.assertTrue(any(predicate in query for query in queries), (path, predicate))
        # The seal is last: semantic mutation tests disable it to prove pool,
        # relation and ownership predicates are not passing solely via hashes.
        if check_seal:
            actual = hashlib.sha256(json.dumps(tokens, separators=(",", ":")).encode()).hexdigest()
            self.assertEqual(actual, spec["sha256"], path + ": reviewed borrowed body drift")

    def test_registered_borrowed_sql_cannot_expand_authority(self):
        for path, spec in REVIEWED_BORROWED_SQL.items():
            source = (ROOT / path).read_text()
            self.assert_extension_storage(path, source)
            for extra in ('func extra() { db.ExecContext(ctx, "DELETE FROM lmm_other.records") }',
                          'func extra() { sql.Open("postgres", input) }',
                          'func extra() { borrowed := db; borrowed.Close() }',
                          'var escape = db.QueryContext'):
                with self.subTest(path=path, extra=extra), self.assertRaises(AssertionError):
                    self.assert_registered_borrowed_sql(path, source + "\n" + extra, check_seal=False)
            for predicate in spec["predicates"]:
                changed = source.replace(predicate, "TRUE", 1)
                self.assertNotEqual(changed, source, (path, predicate))
                with self.subTest(path=path, predicate=predicate), self.assertRaises(AssertionError):
                    self.assert_registered_borrowed_sql(path, changed, check_seal=False)
            with self.subTest(path=path, unregistered=True), self.assertRaises(AssertionError):
                self.assert_extension_storage(path.replace(".go", "_unregistered.go"), source)

    def test_reviewed_owner_closures_preserve_authority_and_replay_guards(self):
        for path in REVIEWED_OWNER_BOUNDARIES:
            source = (ROOT / path).read_text()
            self.assert_extension_storage(path, source)
            if path.endswith("freshinstall/payments_upgrade.go"):
                old, new = 'receipt.Status != "already_committed"', "false"
            elif "/payments/" in path:
                old, new = "after != before", "false"
            elif path.endswith("freshinstall/checkin_policy.go"):
                old, new = "current_user<>$3", "true"
            elif "/operations/" in path:
                old, new = "operation_id=$1", "true"
            else:
                old, new = "current != expected", "false"
            changed = source.replace(old, new, 1)
            self.assertNotEqual(changed, source)
            with self.subTest(owner=path, removed=old), self.assertRaises(AssertionError):
                self.assert_reviewed_owner_boundary(path, changed, check_seal=False)
            with self.subTest(owner=path, unregistered=True), self.assertRaises(AssertionError):
                self.assert_extension_storage(path.replace(".go", "_unregistered.go"), source)

    def test_operations_owner_predicates_without_body_seals(self):
        self.assertEqual(set(OWNER_EVIDENCE), set(REVIEWED_OWNER_BOUNDARIES))
        for path in REVIEWED_OWNER_BOUNDARIES:
            if "/operations/" not in path:
                continue
            source = (ROOT / path).read_text()
            for guard in OWNER_OPERATIONS_GUARDS:
                changed = source.replace(guard, "REMOVED_OWNER_GUARD", 1)
                self.assertNotEqual(changed, source, guard)
                with self.subTest(owner=path, guard=guard), self.assertRaises(AssertionError):
                    self.assert_reviewed_owner_boundary(path, changed, check_seal=False)
            # Extra SQL and a new connection must fail even with all predicates.
            for extra in ('tx.ExecContext(ctx, "DELETE FROM core_billing.accounts")',
                          'sql.Open("postgres", "unapproved")'):
                with self.subTest(owner=path, extra=extra), self.assertRaises(AssertionError):
                    self.assert_extension_storage(path, source + "\nfunc bad() { " + extra + " }")

    def test_pending_owner_closure_is_not_reachable_from_runtime(self):
        seen = set()
        for source in (ROOT / "services/extensions").rglob("*.go"):
            if source.name.endswith("_test.go"):
                continue
            path = source.relative_to(ROOT).as_posix()
            tokens = go_tokens(source.read_text())
            for symbol, callers in OWNER_ENTRY_CALLERS.items():
                if symbol in tokens:
                    seen.add(symbol)
                    self.assertIn(path, callers, (path, symbol))
        self.assertEqual(seen, set(OWNER_ENTRY_CALLERS))
        for path in PENDING_OWNER_BOUNDARIES:
            with self.subTest(pending=path), self.assertRaises(AssertionError):
                self.assert_extension_storage(path, (ROOT / path).read_text())
        for path in REVIEWED_BORROWED_SQL:
            source = (ROOT / path).read_text()
            for symbol in OWNER_ENTRY_CALLERS:
                with self.subTest(path=path, owner=symbol), self.assertRaises(AssertionError):
                    call = ('exec.Command("extensions", ' + symbol + ').Run()'
                            if symbol.startswith('"') else symbol + '()')
                    self.assert_extension_storage(path, source + "\nfunc bad() { " + call + " }")

    def test_store_read_adapters_reject_writes_and_extra_calls(self):
        for path, (method, query) in STORE_READ_QUERIES.items():
            source = (ROOT / path).read_text()
            self.assert_extension_storage(path, source)
            for changed in (
                source.replace(query, '`DELETE FROM lmm_store.orders RETURNING id`'),
                source.replace(query, '`WITH removed AS (DELETE FROM lmm_store.orders RETURNING *) SELECT * FROM removed`'),
                source.replace("t.tx." + method, "t.tx.ExecContext"),
                source + '\nfunc unsafeBoundary() { t.tx.ExecContext(t.ctx, `DELETE FROM lmm_store.orders`) }',
                source + '\nfunc unsafeBoundary() { t.tx.Begin() }',
            ):
                self.assertNotEqual(changed, source)
                with self.subTest(path=path), self.assertRaises(AssertionError):
                    self.assert_extension_storage(path, changed)

    def assert_assistant_composition(self, text):
        for forbidden in ("github.com/lib/pq", "gorm.io", "go-sql-driver", "jackc/pgx", "sqlite"):
            self.assertNotIn(forbidden, text, COMPOSITION_ASSISTANT)
        self.assertNotRegex(text, r"\bsql\s*\.\s*(?:Open|OpenDB)\s*\(", COMPOSITION_ASSISTANT)
        self.assertNotRegex(text, r"\bdb\s*\.", COMPOSITION_ASSISTANT)
        self.assert_no_close_declaration(go_tokens(text))
        self.assertIn("func (r *Runtime) composeAssistantDelegation(", text, COMPOSITION_ASSISTANT)
        for schema in CORE_SCHEMAS:
            self.assertNotIn(schema, text, COMPOSITION_ASSISTANT)
        self.assertNotRegex(text, r"\bcore_[a-z0-9_]+\.", COMPOSITION_ASSISTANT)
        self.assertNotRegex(text, r"\blmm_[a-z0-9_]+\.", COMPOSITION_ASSISTANT)
        self.assertNotRegex(text, r"(?i)\b(?:FROM|JOIN|UPDATE|INTO|TRUNCATE)\s+(?:public\.)?(?:core_|lmm_|payment_|extension_)", COMPOSITION_ASSISTANT)

    def test_unregistered_storage_files_cannot_import_driver_or_open_pools(self):
        for path in ("services/extensions/internal/app/unapproved.go",
                     "services/extensions/internal/composition/unapproved.go",
                     "services/extensions/internal/modules/store/postgres.go"):
            source = (ROOT / path).read_text() if (ROOT / path).is_file() else "package example\n"
            for forbidden in ('import _ "github.com/lib/pq"', 'sql.Open("postgres", dsn)',
                              'sql.OpenDB(connector)', 'sql . Open ("postgres", dsn)'):
                with self.subTest(path=path, forbidden=forbidden):
                    with self.assertRaises(AssertionError):
                        self.assert_extension_storage(path, source + "\n" + forbidden)
        with self.assertRaises(AssertionError):
            self.assert_extension_storage("services/extensions/internal/composition/unapproved.go",
                                          'package composition\nimport "database/sql"')
        assistant_source = (ROOT / COMPOSITION_ASSISTANT).read_text()
        for forbidden in ('import _ "github.com/lib/pq"', 'sql.Open("postgres", dsn)',
                          'sql.OpenDB(connector)', 'sql . Open ("postgres", dsn)',
                          'SELECT * FROM core_billing.balance_state',
                          'SELECT * FROM core_identity.users',
                          'SELECT * FROM core_payments.intents',
                          'SELECT * FROM core_oauth.families',
                          'SELECT * FROM lmm_operations.publications',
                          'func (r *Runtime) Close() error { return nil }',
                          'func bad() { db.Query("SELECT 1") }'):
            with self.subTest(path=COMPOSITION_ASSISTANT, forbidden=forbidden):
                with self.assertRaises(AssertionError):
                    self.assert_extension_storage(COMPOSITION_ASSISTANT, assistant_source + "\n" + forbidden)

    def test_composition_storage_guards_cannot_be_removed_or_gain_table_access(self):
        source = (ROOT / COMPOSITION_STORAGE).read_text()
        self.assert_extension_storage(COMPOSITION_STORAGE, source)
        for required in COMPOSITION_STORAGE_GUARDS:
            self.assertIn(required, source)
            with self.subTest(required=required), self.assertRaises(AssertionError):
                self.assert_extension_storage(COMPOSITION_STORAGE, source.replace(required, ""))
        for forbidden in ("core_history.sources", "core_history.log_records",
                          "core_identity.users", "core_billing.balance_state", "core_events.outbox",
                          "lmm_store.orders", "lmm_support.messages", "lmm_assistant.records",
                          "lmm_promotions.records", "payment_orders", 'sql.OpenDB(connector)'):
            with self.subTest(forbidden=forbidden), self.assertRaises(AssertionError):
                self.assert_extension_storage(COMPOSITION_STORAGE, source + "\nSELECT * FROM " + forbidden)

    def assert_composition_runtime(self, text):
        for required in COMPOSITION_RUNTIME_GUARDS:
            self.assertIn(required, text)
        tokens = go_tokens(text)
        expected_pools = ("Checkins", "Store", "Support", "Operations", "Community", "Acquisition")
        calls = [tokens[i + 2:i + 8] for i in range(len(tokens) - 1)
                 if tokens[i:i + 2] == ["openStorage", "("]]
        self.assertCountEqual(calls, [go_tokens(f"ctx, cfg.{name})") for name in expected_pools])
        bundled = [i for i in range(len(tokens) - 1) if tokens[i:i + 2] == ["openStorageBundle", "("]]
        self.assertEqual(len(bundled), 1)
        owned_close = go_tokens("r.close = append(r.close, db.Close)")
        self.assertEqual(sum(tokens[i:i + len(owned_close)] == owned_close
                             for i in range(len(tokens))), len(calls) + len(bundled))
        body = self.go_function_body(
            text, "func New(ctx context.Context, cfg Config) (_ *Runtime, resultErr error)")
        self.token_index(body, """
            db, e := openStorage(ctx, cfg.Checkins)
            if e != nil { return nil, e }
            r.close = append(r.close, db.Close)
        """)
        for component in ("Store", "Support", "Operations", "Community", "Acquisition"):
            self.token_index(body, f"""
                db, err := openStorage(ctx, cfg.{component})
                if err != nil {{ return nil, err }}
                r.close = append(r.close, db.Close)
            """)
        self.token_index(body, OPERATIONS_RUNTIME_BLOCK)
        self.token_index(body, "defer func() { if resultErr != nil { _ = r.Close() } }()")
        close = self.go_function_body(text, "func (r *Runtime) Close() error")
        self.token_index(close, """
            for i := len(r.close) - 1; i >= 0; i-- {
                err = errors.Join(err, r.close[i]())
            }
            r.close = nil
        """)

    def token_index(self, tokens, source):
        expected = go_tokens(source)
        positions = [i for i in range(len(tokens) - len(expected) + 1)
                     if tokens[i:i + len(expected)] == expected]
        self.assertEqual(len(positions), 1, source)
        return positions[0]

    def braced_tokens(self, tokens, opening):
        self.assertEqual(tokens[opening], "{")
        depth = 1
        for end in range(opening + 1, len(tokens)):
            if tokens[end] == "{":
                depth += 1
            elif tokens[end] == "}":
                depth -= 1
                if depth == 0:
                    return tokens[opening + 1:end]
        self.fail("unterminated token block")

    def go_function_body(self, source, signature):
        tokens = go_tokens(source)
        header = go_tokens(signature + " {")
        start = self.token_index(tokens, signature + " {") + len(header)
        depth = 1
        for end in range(start, len(tokens)):
            if tokens[end] == "{":
                depth += 1
            elif tokens[end] == "}":
                depth -= 1
                if depth == 0:
                    return tokens[start:end]
        self.fail("unterminated Go guard function: " + signature)

    def assert_named_go_import(self, text, package):
        tokens = go_tokens(text)
        imports = self.token_index(tokens, "import (") + 2
        end = tokens.index(")", imports)
        imported = self.token_index(tokens[imports:end], json.dumps(package)) + imports
        self.assertTrue(tokens[imported - 1] == "(" or tokens[imported - 1].startswith('"'))
        return tokens

    def assert_no_close_declaration(self, tokens):
        for index, token in enumerate(tokens[:-1]):
            if token != "func":
                continue
            name = index + 1
            if tokens[name] == "(":
                name = tokens.index(")", name) + 1
            self.assertNotEqual(tokens[name], "Close")

    def assert_sql_selectors(self, tokens, expected):
        selectors = [tokens[i + 2] for i in range(len(tokens) - 2)
                     if tokens[i:i + 2] == ["sql", "."]]
        self.assertEqual(selectors, expected)

    def assert_operations_postgres(self, text):
        tokens = self.assert_named_go_import(text, "github.com/lib/pq")
        self.assert_named_go_import(text, "database/sql")
        self.assert_sql_selectors(tokens, ["DB", "DB", "DB", "Tx", "Tx"])
        # The existing default named pq import is allowed for the owner-only
        # quoting API. Blank/aliased imports and every connector API stay barred.
        owner = self.go_function_body(
            text, "func Initialize(ctx context.Context, owner *sql.DB, runtimeRole string) error")
        expected = [go_tokens("pq.QuoteIdentifier(runtimeRole)"),
                    go_tokens("pq.QuoteIdentifier(database)"),
                    go_tokens("pq.QuoteIdentifier(database)")]
        for scope in (tokens, owner):
            calls = [scope[i:i + 6] for i in range(len(scope) - 1)
                     if scope[i:i + 2] == ["pq", "."]]
            self.assertEqual(calls, expected)
        constructor = self.go_function_body(
            text, "func NewPostgres(ctx context.Context, db *sql.DB) (*Releases, error)")
        self.assertEqual(constructor, go_tokens("""
            if err := Check(ctx, db); err != nil { return nil, err }
            return &Releases{db}, nil
        """))
        self.assertFalse(any(tokens[i:i + 2] == [".", "Close"] for i in range(len(tokens) - 1)))
        self.assert_no_close_declaration(tokens)
        seal = self.go_function_body(text, "func SealInstalledSchema(ctx context.Context, tx *sql.Tx) error")
        self.assertEqual(seal, go_tokens("return sealInstalledSchemaVersion(ctx, tx, 3)"))
        seal_version = self.go_function_body(
            text, "func sealInstalledSchemaVersion(ctx context.Context, tx *sql.Tx, version int) error")
        self.assertEqual(seal_version, go_tokens("""
            var fingerprint string
            if err := tx.QueryRowContext(ctx, fingerprintSQL).Scan(&fingerprint); err != nil { return err }
            result, err := tx.ExecContext(ctx, "UPDATE lmm_operations.contract SET fingerprint=$1 WHERE version=$2", fingerprint, version)
            if err != nil { return err }
            count, err := result.RowsAffected()
            if err != nil || count != 1 { return ErrUnavailable }
            return nil
        """))
        check = self.go_function_body(text, "func Check(ctx context.Context, runtime *sql.DB) error")
        self.token_index(check, "runtime.QueryRowContext(ctx, isolatedSQL+")
        self.token_index(check, "runtime.QueryRowContext(ctx, fingerprintSQL).Scan(&actual)")
        self.token_index(check, "installed != actual || len(actual) != 32")
        self.token_index(check, 'runtime.QueryRowContext(ctx, "SELECT fingerprint FROM lmm_operations.contract WHERE version=3")')
        self.token_index(check, 'tables := []string{"contract", "publications", "reads", "rss_config", "script_repository"}')
        self.token_index(check, 'if ionet { tables = append(tables, "ionet_config", "ionet_operations") }')
        self.token_index(check, 'for _, table := range tables {')
        self.token_index(check, 'table == "publications" || table == "reads" || table == "ionet_operations", table == "rss_config" || table == "script_repository" || table == "ionet_config" || table == "ionet_operations"')
        literals = [token[1:-1] for token in check if token.startswith(('"', '`'))]
        for required in OPERATIONS_CHECK_GUARDS:
            self.assertTrue(any(required in literal for literal in literals), required)

    def assert_operations_releases(self, text):
        tokens = self.assert_named_go_import(text, "database/sql")
        self.assert_sql_selectors(tokens, ["DB", "ErrNoRows", "ErrNoRows"])
        closes = [tokens[i - 1:i + 2] for i in range(1, len(tokens) - 1)
                  if tokens[i:i + 2] == [".", "Close"]]
        self.assertEqual(closes, [["rows", ".", "Close"]])
        listing = self.go_function_body(text, "func (s *Releases) List(ctx context.Context, limit int) ([]ReleaseNote, error)")
        self.token_index(listing, "defer rows.Close()")
        # Pin the complete reviewed query/scan lifetime. A token named rows can
        # otherwise shadow the query result with the caller's sql.DB and close
        # that pool while satisfying the same Close selector/name checks.
        self.assertEqual(listing, go_tokens(OPERATIONS_LIST_BODY))
        self.assert_no_close_declaration(tokens)

    def assert_script_repository_storage(self, text):
        tokens = self.assert_named_go_import(text, "database/sql")
        self.assert_no_close_declaration(tokens)
        constructor = self.go_function_body(
            text, "func NewScriptRepository(db *sql.DB) *ScriptRepository")
        self.assertEqual(constructor, go_tokens(
            "return &ScriptRepository{db: db, client: newRSS(nil).client}"))
        # Count selectors, not just calls: aliases/method values cannot smuggle
        # an extra DB operation past the literal-call allowlist. Each approved
        # query must occur inside its named method on the injected s.db pool.
        raw = go_tokens("snapshot.Raw(name)")
        raw_index = self.token_index(tokens, "snapshot.Raw(name)")
        self.assertEqual(tokens[raw_index:raw_index+len(raw)], raw)
        methods = [tokens[i+1] for i in range(len(tokens)-1)
                   if tokens[i] == "." and tokens[i+1] in SQL_METHODS
                   and i != raw_index+1]
        self.assertEqual(methods, ["QueryRowContext"] * len({method for method, _ in OPERATIONS_REPOSITORY_QUERIES}))
        for method, query in OPERATIONS_REPOSITORY_QUERIES:
            signature = re.search(
                r"(?m)^func \(s \*ScriptRepository\) " + method + r"\([^\n]+\) \([^\n]+\) \{", text)
            self.assertIsNotNone(signature, method)
            body = self.go_function_body(text, signature.group(0)[:-2])
            if method == "SaveWithCredentials":
                normal, sealed = [q for m, q in OPERATIONS_REPOSITORY_QUERIES if m == method]
                self.token_index(body, "query := `" + normal + "`")
                self.token_index(body, "args := []any{settings.URL, settings.Branch, actor, settings.Revision}")
                self.token_index(body, 'if token != "" || clear { query = `' + sealed + '` args = append(args, sealed) }')
                self.token_index(body, "s.db.QueryRowContext(ctx, query, args...)")
                # Three uses only: initializer, the one controlled replacement,
                # and the borrowed-pool call. No additional dynamic SQL producer.
                self.assertEqual(body.count("query"), 3)
                self.assertEqual(body.count("args"), 4)
            else:
                call = go_tokens("s.db.QueryRowContext(ctx, `" + query + "`")
                index = self.token_index(body, "s.db.QueryRowContext(ctx, `" + query + "`")
                self.assertIn(body[index+len(call)], {",", ")"}, method)
            self.assertEqual(sum(body[i:i+2] == [".", "QueryRowContext"]
                                 for i in range(len(body)-1)), 1, method)

    def assert_script_repository_schema(self, text):
        def sql_tokens(sql):
            return [token for token in re.findall(r"--[^\n]*|" + GO_TOKEN.pattern, sql)
                    if not token.startswith(("--", "//", "/*"))]
        # Owner creates and seeds exactly one singleton; runtime only reads and
        # CAS-updates it. Snapshot metadata is atomic with the imported body.
        self.assertEqual(sql_tokens(text), sql_tokens("""
            CREATE TABLE lmm_operations.script_repository (
             singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
             revision bigint NOT NULL DEFAULT 0 CHECK (revision BETWEEN 0 AND 9007199254740991),
             repository_url text NOT NULL DEFAULT '' CHECK (octet_length(repository_url)<=256),
             branch text NOT NULL DEFAULT 'main' CHECK (octet_length(branch) BETWEEN 1 AND 200),
             scripts jsonb CHECK (jsonb_typeof(scripts)='object' AND octet_length(scripts::text)<=12582912),
             snapshot_repository_url text NOT NULL DEFAULT '' CHECK (octet_length(snapshot_repository_url)<=256),
             snapshot_branch text NOT NULL DEFAULT '' CHECK (octet_length(snapshot_branch)<=200),
             snapshot_pulled_at timestamptz,
             updated_by bigint CHECK (updated_by BETWEEN 1 AND 9007199254740991),
             CHECK ((scripts IS NULL AND snapshot_repository_url='' AND snapshot_branch='' AND snapshot_pulled_at IS NULL)
             OR (scripts IS NOT NULL AND snapshot_repository_url<>'' AND snapshot_branch<>'' AND snapshot_pulled_at IS NOT NULL))
            );
            INSERT INTO lmm_operations.script_repository(singleton) VALUES (true);
            REVOKE ALL ON lmm_operations.script_repository FROM PUBLIC;
        """))

    def assert_script_repository_install_grants(self, text):
        compiled = self.go_function_body(text, "func compiled(component string) (plan, error)")
        start = self.token_index(compiled, 'case "operations":')
        end = compiled.index("case", start + 1)
        self.assertEqual(compiled[start:end], go_tokens('''
            case "operations":
                return plan{prefix: "lmm_operations", schema: operations.Schema, namespaces: []string{"lmm_operations"}, grants: []string{
                    "GRANT SELECT ON lmm_operations.contract TO ",
                    "GRANT SELECT,INSERT ON lmm_operations.publications,lmm_operations.reads TO ",
                    "GRANT SELECT,UPDATE ON lmm_operations.rss_config,lmm_operations.script_repository,lmm_operations.ionet_config TO ",
                    "GRANT SELECT,INSERT,UPDATE ON lmm_operations.ionet_operations TO ",
                    "GRANT SELECT,USAGE ON SEQUENCE lmm_operations.publication_id TO ",
                }}, nil
        '''))

    def assert_operations_upgrade_dispatch(self, text):
        for wrapper, mode in (("RunOperationsUpgradeCLI", "upgradeOperationsBase"),
                              ("RunScriptRepositoryCredentialsUpgradeCLI", "upgradeScriptCredentials"),
                              ("RunIoNetUpgradeCLI", "upgradeIoNet")):
            self.assertEqual(self.go_function_body(text,
                f"func {wrapper}(ctx context.Context, args []string, out io.Writer) error"),
                go_tokens(f"return runOperationsUpgradeCLI(ctx, args, out, {mode})"))
        body = self.go_function_body(text,
            "func runOperationsUpgradeCLI(ctx context.Context, args []string, out io.Writer, mode operationsUpgradeMode) error")
        self.token_index(body, "credentials := mode == upgradeScriptCredentials")
        self.token_index(body, "ionet := mode == upgradeIoNet")
        self.token_index(body, 'if !credentials && !ionet { f.IntVar(&targetVersion, "target-version", 2, "2 for RSS, 3 for script repository") }')
        self.token_index(body, """
            if f.Parse(args) != nil || f.NArg() != 0 || (!credentials && !ionet && targetVersion != 2 && targetVersion != 3) {
                return ErrRejected
            }
        """)
        self.token_index(body, 'c.Component = "operations"')
        self.token_index(body, "dsn, err := ownerURL(c, p)")
        self.token_index(body, "db.SetMaxOpenConns(1)")
        self.token_index(body, "context.WithTimeout(ctx, 30*time.Second)")
        self.token_index(body, "defer tx.Rollback()")
        fence = self.token_index(body, '''
            err = tx.QueryRowContext(ctx, `SELECT current_database()=$1 AND pg_get_userbyid(d.datdba)=$2 AND session_user<>$3
 AND EXISTS(SELECT FROM pg_roles WHERE rolname=$2 AND NOT rolcanlogin)
 AND NOT EXISTS(SELECT FROM pg_roles WHERE rolname IN(session_user,$2,$3) AND (rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls))
 AND NOT EXISTS(SELECT FROM pg_auth_members WHERE member=(SELECT oid FROM pg_roles WHERE rolname=$3))
 AND NOT EXISTS(SELECT FROM pg_roles WHERE rolcanlogin AND NOT rolsuper AND rolname<>session_user
   AND (has_database_privilege(oid,d.oid,'CONNECT') OR pg_has_role(oid,$2,'SET') OR pg_has_role(oid,$3,'SET')))
 FROM pg_database d WHERE datname=current_database()`, c.ExpectedDatabase, c.OwnerRole, c.RuntimeRole).Scan(&allowed)
            if err != nil || !allowed { return ErrRejected }
        ''')
        role = self.token_index(body, '''
            if _, err := tx.ExecContext(ctx, "SET LOCAL ROLE "+pq.QuoteIdentifier(c.OwnerRole)); err != nil {
                return ErrRejected
            }
        ''')
        dispatch = self.token_index(body, """
            if ionet {
                err = operations.UpgradeIoNet(ctx, tx, c.RuntimeRole, operation, recoverOnly)
            } else if credentials {
                err = operations.UpgradeScriptRepositoryCredentials(ctx, tx, c.RuntimeRole, operation, recoverOnly)
            } else if targetVersion == 3 {
                err = operations.UpgradeScriptRepository(ctx, tx, c.RuntimeRole, operation, recoverOnly)
            } else {
                err = operations.UpgradeRSS(ctx, tx, c.RuntimeRole, operation, recoverOnly)
            }
            if err != nil { return err }
            if !recoverOnly {
                if err := tx.Commit(); err != nil { return ErrCommitUnknown }
            }
            receipt := map[string]any{"database": c.ExpectedDatabase, "operation_id": operation, "version": targetVersion}
            if ionet { receipt["version"] = 1
 receipt["component"] = "ionet" }
            if credentials { receipt["version"] = 1
 receipt["component"] = "script_repository_credentials" }
            if err := json.NewEncoder(out).Encode(receipt); err != nil { return ErrCommitUnknown }
            return nil
        """)
        self.assertLess(fence, role)
        self.assertLess(role, dispatch)
        self.assertEqual(body.count("UpgradeRSS"), 1)
        self.assertEqual(body.count("UpgradeScriptRepository"), 1)
        self.assertEqual(body.count("UpgradeScriptRepositoryCredentials"), 1)
        self.assertEqual(body.count("UpgradeIoNet"), 1)

    def test_operations_owner_target_dispatch_and_version_seals(self):
        source = (ROOT / OFFLINE_OPERATIONS_CLI).read_text()
        # Direct semantic checks do not depend on a freshly calculated hash.
        self.assert_operations_upgrade_dispatch(source)
        for old, new in (
            ('"target-version", 2', '"target-version", 3'),
            ('targetVersion != 2 && targetVersion != 3', 'targetVersion != 2 && targetVersion != 4'),
            ('targetVersion == 3', 'targetVersion == 2'),
            ('err = operations.UpgradeRSS(', 'err = operations.UpgradeScriptRepository('),
            ('c.RuntimeRole, operation, recoverOnly', 'c.OwnerRole, operation, recoverOnly'),
            ('c.RuntimeRole, operation, recoverOnly', 'c.RuntimeRole, operation, false'),
            ('if !recoverOnly {', 'if true {'),
            ('"version": targetVersion', '"version": 2'),
            ('session_user<>$3', 'true'),
            ('NOT rolcanlogin', 'true'),
            ('defer tx.Rollback()', ''),
            ('return ErrCommitUnknown', 'return nil'),
        ):
            changed = source.replace(old, new, 1)
            self.assertNotEqual(changed, source, old)
            with self.subTest(dispatch=old), self.assertRaises(AssertionError):
                self.assert_operations_upgrade_dispatch(changed)
        postgres = (ROOT / OPERATIONS_POSTGRES).read_text()
        self.assert_operations_postgres(postgres)
        for old, new in (
            ('return sealInstalledSchemaVersion(ctx, tx, 3)', 'return sealInstalledSchemaVersion(ctx, tx, 2)'),
            ('WHERE version=$2', ''),
            ('count != 1', 'count < 0'),
            ('result.RowsAffected()', 'int64(1), error(nil)'),
        ):
            changed = postgres.replace(old, new, 1)
            self.assertNotEqual(changed, postgres, old)
            with self.subTest(seal=old), self.assertRaises(AssertionError):
                self.assert_operations_postgres(changed)

    def test_script_repository_queries_are_exact_borrowed_and_cas_only(self):
        source = (ROOT / OPERATIONS_REPOSITORY).read_text()
        self.assert_extension_storage(OPERATIONS_REPOSITORY, source)
        for _, query in OPERATIONS_REPOSITORY_QUERIES:
            changes = [source.replace("`" + query + "`", "`" + unsafe + "`", 1)
                       for unsafe in (
                           "SELECT * FROM core_identity.users",
                           'UPDATE "core_billing".balance_state SET balance=0',
                           "SELECT * FROM lmm_operations.publications",
                           "DELETE FROM lmm_operations.script_repository RETURNING revision",
                           "WITH gone AS (DELETE FROM lmm_operations.script_repository RETURNING *) SELECT * FROM gone",
                           "ALTER TABLE lmm_operations.script_repository ADD COLUMN unsafe text",
                           "GRANT ALL ON lmm_operations.script_repository TO PUBLIC",
                           query + "; DROP TABLE lmm_operations.script_repository",
                       )]
            if "AND revision=" in query:
                changes.append(source.replace(query, re.sub(r" AND revision=\$[0-9]+", "", query), 1))
            for changed in changes:
                self.assertNotEqual(changed, source)
                with self.subTest(query=query, changed=changed), self.assertRaises(AssertionError):
                    self.assert_extension_storage(OPERATIONS_REPOSITORY, changed)
        for extra in (
            's.db.ExecContext(ctx, `DELETE FROM lmm_operations.script_repository`)',
            's.db.ExecContext(ctx, `CREATE TABLE lmm_operations.extra(id int)`)',
            's.db.QueryRowContext(ctx, dynamicSQL)',
            'query := s.db.QueryRowContext; query(ctx, dynamicSQL)',
            's.db.PrepareContext(ctx, dynamicSQL)',
            's.db.BeginTx(ctx, nil)',
            's.db.Conn(ctx)',
            'driver := s.db.Driver',
            's.db.Close()',
            'borrowed := s.db; alias := borrowed; closePool := alias.Close; closePool()',
            'sql.Open("postgres", dsn)',
            'var opener = sql.OpenDB',
            'operations.UpgradeScriptRepository(ctx, tx, role, operation, false)',
            'operations.SealInstalledSchema(ctx, tx)',
        ):
            with self.subTest(extra=extra), self.assertRaises(AssertionError):
                self.assert_extension_storage(OPERATIONS_REPOSITORY,
                    source + "\nfunc unsafeBoundary() { " + extra + " }")
        for changed in (
            source.replace("s.db.QueryRowContext", "other.QueryRowContext", 1),
            source.replace('"database/sql"', 'dbsql "database/sql"').replace("sql.", "dbsql."),
            source + "\nfunc (s *ScriptRepository) Close() error { return nil }",
        ):
            self.assertNotEqual(changed, source)
            with self.subTest(pool_boundary=changed), self.assertRaises(AssertionError):
                self.assert_extension_storage(OPERATIONS_REPOSITORY, changed)
        with self.assertRaises(AssertionError):
            self.assert_extension_storage(OPERATIONS_REPOSITORY.replace(".go", "_unregistered.go"), source)

    def test_script_repository_fetch_has_no_database_authority(self):
        source = (ROOT / OPERATIONS_REPOSITORY_FETCH).read_text()
        self.assert_extension_storage(OPERATIONS_REPOSITORY_FETCH, source)
        for extra in (
            's.db.QueryRowContext(ctx, `SELECT * FROM lmm_operations.script_repository`)',
            's.db.ExecContext(ctx, `UPDATE core_future.records SET body=NULL`)',
            's.db.ExecContext(ctx, `DELETE FROM lmm_operations.script_repository`)',
            's.db.ExecContext(ctx, `CREATE TABLE lmm_operations.extra(id int)`)',
            'query := s.db.QueryRowContext',
            'borrowed := s.db; borrowed.Close()',
            'sql.Open("postgres", dsn)',
            'operations.UpgradeScriptRepository(ctx, tx, role, operation, false)',
        ):
            with self.subTest(extra=extra), self.assertRaises(AssertionError):
                self.assert_extension_storage(OPERATIONS_REPOSITORY_FETCH,
                    source + "\nfunc unsafeBoundary() { " + extra + " }")

    def test_script_repository_owner_upgrade_is_frozen_and_not_runtime_callable(self):
        source = (ROOT / OFFLINE_REPOSITORY_UPGRADE).read_text()
        self.assert_extension_storage(OFFLINE_REPOSITORY_UPGRADE, source)
        for old, new in (
            ('NOT rolcanlogin', 'true'),
            ('actual != installed', 'false'),
            ('version != 2', 'version != 1'),
            ('if recoverOnly {', 'if false {'),
            ('operation_id=$1', 'true'),
            ('installed_by=session_user', 'true'),
            ('a.grantee<>c.relowner', 'false'),
            ('GRANT SELECT,UPDATE ON lmm_operations.script_repository TO ',
             'GRANT ALL ON lmm_operations.script_repository TO '),
            ('return SealInstalledSchema(ctx, tx)', 'return nil'),
        ):
            changed = source.replace(old, new, 1)
            self.assertNotEqual(changed, source, old)
            with self.subTest(owner=old), self.assertRaises(AssertionError):
                self.assert_extension_storage(OFFLINE_REPOSITORY_UPGRADE, changed)
        rss = (ROOT / OFFLINE_RSS_UPGRADE).read_text()
        self.token_index(go_tokens(rss), 'return sealInstalledSchemaVersion(ctx, tx, 2)')
        for path in (OPERATIONS_REPOSITORY, OPERATIONS_REPOSITORY_FETCH,
                     OPERATIONS_INJECTION, OPERATIONS_RSS,
                     "services/extensions/internal/app/run.go"):
            runtime = (ROOT / path).read_text()
            for call in (
                'operations.UpgradeScriptRepository(ctx, tx, role, operation, false)',
                'operations.SealInstalledSchema(ctx, tx)',
                'sealInstalledSchemaVersion(ctx, tx, 2)',
                'freshinstall.RunOperationsUpgradeCLI(ctx, args, out)',
            ):
                with self.subTest(runtime=path, call=call), self.assertRaises(AssertionError):
                    self.assert_extension_storage(path, runtime + "\nfunc unsafeBoundary() { " + call + " }")

    def test_script_repository_schema_and_owner_grants_are_narrow(self):
        path = ROOT / "services/extensions/internal/modules/operations/script_repository_schema.sql"
        schema = path.read_text()
        self.assert_script_repository_schema(schema)
        for changed in (
            schema.replace("CHECK (singleton)", "CHECK (true)"),
            schema.replace("INSERT INTO lmm_operations.script_repository(singleton) VALUES (true);", ""),
            schema.replace("REVOKE ALL", "GRANT ALL"),
            schema.replace("revision BETWEEN 0 AND 9007199254740991", "revision>=0"),
            schema.replace("snapshot_pulled_at IS NOT NULL", "true"),
            schema + "\nUPDATE core_identity.users SET platform_role='superadmin';",
            schema + "\nGRANT SELECT ON lmm_operations.script_repository TO PUBLIC;",
            schema + "\nCREATE TABLE lmm_operations.extra(id int);",
        ):
            self.assertNotEqual(changed, schema)
            with self.subTest(schema=changed), self.assertRaises(AssertionError):
                self.assert_script_repository_schema(changed)
        installer = (ROOT / OFFLINE_INSTALLER).read_text()
        self.assert_script_repository_install_grants(installer)
        grant = "GRANT SELECT,UPDATE ON lmm_operations.rss_config,lmm_operations.script_repository,lmm_operations.ionet_config TO "
        for privilege in ("ALL", "SELECT,UPDATE,INSERT", "SELECT,UPDATE,DELETE", "SELECT,UPDATE,TRUNCATE", "SELECT,UPDATE,MAINTAIN"):
            changed = installer.replace(grant, grant.replace("SELECT,UPDATE", privilege), 1)
            self.assertNotEqual(changed, installer)
            # Exercise the semantic grant check directly, so an old whole-file
            # fingerprint cannot mask a missing privilege assertion.
            with self.subTest(privilege=privilege), self.assertRaises(AssertionError):
                self.assert_script_repository_install_grants(changed)
        storage = (ROOT / COMPOSITION_STORAGE).read_text()
        self.assert_operations_storage(storage)
        policy = 'add("lmm_operations.script_repository", "SELECT", "UPDATE")'
        for replacement in ("", policy.replace('"UPDATE"', '"UPDATE", "DELETE"'),
                            policy.replace('"UPDATE"', '"UPDATE", "INSERT"'),
                            policy.replace("lmm_operations", "core_operations")):
            changed = storage.replace(policy, replacement, 1)
            self.assertNotEqual(changed, storage)
            with self.subTest(policy=replacement), self.assertRaises(AssertionError):
                self.assert_operations_storage(changed)

    def assert_operations_injection(self, text):
        tokens = self.assert_named_go_import(text, "database/sql")
        self.assert_sql_selectors(tokens, ["DB"])
        self.token_index(tokens, "type Config struct { Releases *sql.DB Scripts *ScriptConfig }")
        body = self.go_function_body(text, "func New(ctx context.Context, c Config) (*Service, error)")
        self.assertEqual(body, go_tokens(OPERATIONS_NEW_BODY))
        availability = self.go_function_body(text, "func (s *Service) Availability() Availability")
        self.assertEqual(availability, go_tokens("""
            if s == nil { return Availability{} }
            return Availability{s.Releases != nil, s.Scripts != nil || s.Repository != nil, s.Repository != nil, s.RSS != nil}
        """))
        # Every pool-field access is in those two complete reviewed bodies;
        # another function cannot query, close, alias or export the injected pool.
        accesses = lambda scope: sum(scope[i:i + 2] == [".", "Releases"]
                                     for i in range(len(scope) - 1))
        self.assertEqual(accesses(tokens), accesses(body) + accesses(availability))
        self.assert_no_close_declaration(tokens)

    def assert_storage_addons(self, text):
        body = self.go_function_body(text,
            "func openStorageBundle(ctx context.Context, c StorageConfig, bundle storageBundle) (*sql.DB, error)")
        self.token_index(body, """
            if c.Component == "operations" {
                var config, journal bool
                if err := db.QueryRowContext(bounded, `SELECT to_regclass('lmm_operations.ionet_config') IS NOT NULL,to_regclass('lmm_operations.ionet_operations') IS NOT NULL`).Scan(&config, &journal); err != nil || config != journal {
                    return nil, errors.New("io.net storage addon incomplete")
                }
                if !config {
                    policy.tables = slices.DeleteFunc(slices.Clone(policy.tables), func(table storageTable) bool {
                        return table.name == "lmm_operations.ionet_config" || table.name == "lmm_operations.ionet_operations"
                    })
                }
            }
        """)
        self.token_index(body, """
            if c.Component == "payments" {
                for _, addon := range [][]string{
                    {"public.payment_stripe_subscription_checkouts", "public.payment_stripe_subscription_inbox"},
                    {"public.payment_pancake_subscription_checkouts", "public.payment_pancake_subscription_inbox", "public.payment_pancake_wallet_inbox"},
                } {
                    installed, err := paymentStorageAddonInstalled(bounded, db, addon)
                    if err != nil { return nil, err }
                    if !installed {
                        policy.tables = slices.DeleteFunc(slices.Clone(policy.tables), func(t storageTable) bool { return slices.Contains(addon, t.name) })
                    }
                }
            }
        """)
        self.assertEqual(self.go_function_body(text,
            "func paymentStorageAddonInstalled(ctx context.Context, db *sql.DB, names []string) (bool, error)"), go_tokens("""
            var present, valid int
            err := db.QueryRowContext(ctx, `SELECT count(*),count(*) FILTER(WHERE c.relkind='r') FROM unnest($1::text[]) t(name) JOIN pg_class c ON c.oid=to_regclass(t.name)`, pq.Array(names)).Scan(&present, &valid)
            if err != nil || present != valid || (present != 0 && present != len(names)) {
                return false, errors.New("payment subscription storage addon incomplete")
            }
            return present == len(names), nil
        """))

    def test_optional_storage_addons_reject_partial_installs_without_hash_shortcut(self):
        source = (ROOT / COMPOSITION_STORAGE).read_text()
        self.assert_storage_addons(source)
        for old, new in (("config != journal", "false"),
                         ("present != valid", "false"),
                         ("present != 0 && present != len(names)", "false"),
                         ("if !installed {", "if installed {"),
                         ("return present == len(names), nil", "return true, nil")):
            changed = source.replace(old, new, 1)
            self.assertNotEqual(changed, source)
            with self.subTest(removed=old), self.assertRaises(AssertionError):
                self.assert_storage_addons(changed)
        plan = (ROOT / OFFLINE_COMMERCE_PLAN).read_text()
        self.assert_extension_storage(OFFLINE_COMMERCE_PLAN, plan)
        for old, new in (("strings.Count(p.schema,", "strings.Count(userSQL,"),
                         ("GRANT SELECT,INSERT,UPDATE ON lmm_support", "GRANT ALL ON lmm_support"),
                         ('return plan{}, ErrRejected', 'return p, nil')):
            changed = plan.replace(old, new, 1)
            self.assertNotEqual(changed, plan)
            with self.subTest(owner_plan=old), self.assertRaises(AssertionError):
                self.assert_extension_storage(OFFLINE_COMMERCE_PLAN, changed)

    def test_semantic_initializer_and_cli_guards_reject_rewiring(self):
        runtime = (ROOT / "services/extensions/internal/composition/runtime.go").read_text()
        payments = (ROOT / COMPOSITION_PAYMENTS).read_text()
        startup = (ROOT / "services/extensions/internal/app/run.go").read_text()
        # A field's location in Config is not authority; its checked value is.
        rearranged = startup.replace("Selected: selected, Socket: socket, TokenFile: tokenFile, HostCredential: credential,",
            "HostCredential: credential, TokenFile: tokenFile, Socket: socket, Selected: selected,")
        self.assertNotEqual(startup, rearranged)
        self.assert_runtime_credentials(runtime, payments, rearranged)
        fake = rearranged.replace("HostCredential: credential,", "HostCredential: nil,") + "\n// HostCredential: credential,"
        with self.assertRaises(AssertionError):
            self.assert_runtime_credentials(runtime, payments, fake)
        for changed in (runtime.replace("openStorage(ctx, cfg.Checkins)", "openStorage(ctx, cfg.Store)"),
                        runtime + "\nfunc bad() { openStorage(ctx, cfg.Store) }"):
            with self.assertRaises(AssertionError):
                self.assert_composition_runtime(changed)
        main = (ROOT / "services/core/src/main.rs").read_text()
        self.assert_core_command_boundary(main)
        for old, new in (("Command::Serve => None,", "Command::Serve => Some(admin::Operation::InitDb),"),
                         ('("init-db", Command::InitDb)', '("init-db", Command::Serve)'),
                         ('Some(operation) => runtime.block_on(admin::run(operation))', 'Some(operation) => runtime.block_on(run_core())')):
            changed = main.replace(old, new, 1)
            self.assertNotEqual(changed, main)
            with self.subTest(dispatch=old), self.assertRaises(AssertionError):
                self.assert_core_command_boundary(changed + "\n// " + old)

    def assert_operations_storage(self, text):
        body = self.go_function_body(
            text, "func openStorageBundle(ctx context.Context, c StorageConfig, bundle storageBundle) (*sql.DB, error)")
        self.token_index(body, OPERATIONS_SEQUENCE_BLOCK)
        self.token_index(body, """
            policy, err := storagePolicyFor(c.Component, bundle)
            if err != nil { return nil, err }
            dsn, err := storageURL(c)
        """)
        self.token_index(body, """
	for _, check := range []struct {
		tables   []storageTable
		function string
		want     bool
	}{
		{policy.tables, "has_table_privilege", true},
		{limits.forbidden, "has_table_privilege", false},
		{limits.forbiddenColumns, "has_any_column_privilege", false},
	} {
		var names, privileges []string
		for _, table := range check.tables {
			for _, privilege := range table.privileges {
				names, privileges = append(names, table.name), append(privileges, privilege)
			}
		}
		if len(names) == 0 {
			continue
		}
		var valid bool
		query := `SELECT NOT EXISTS(SELECT FROM unnest($1::text[],$2::text[]) AS t(name,privilege) WHERE COALESCE(` + check.function + `(current_user,to_regclass(t.name),t.privilege),false)<>$3)`
		if err := db.QueryRowContext(bounded, query, pq.Array(names), pq.Array(privileges), check.want).Scan(&valid); err != nil || !valid {
			return nil, errors.New("extension storage grants differ from runtime contract")
		}
	}
	if c.Component == "assistant" || bundle == toolMarketBuiltinsBundle {
		for _, namespace := range policy.namespaces {
			var allowed bool
			if err := db.QueryRowContext(bounded, `SELECT COALESCE(has_schema_privilege(current_user,$1,'USAGE'),false)`, namespace).Scan(&allowed); err != nil || !allowed {
				return nil, errors.New("extension bundled storage schema grants unavailable")
			}
		}
	}
        """)
        tables = self.go_function_body(text, "func storageTables(component string) []storageTable")
        self.token_index(tables, """
            case "operations":
                add("lmm_operations.contract", "SELECT")
                add("lmm_operations.publications", "SELECT", "INSERT")
                add("lmm_operations.reads", "SELECT", "INSERT")
                add("lmm_operations.rss_config", "SELECT", "UPDATE")
                add("lmm_operations.script_repository", "SELECT", "UPDATE")
                add("lmm_operations.ionet_config", "SELECT", "UPDATE")
                add("lmm_operations.ionet_operations", "SELECT", "INSERT", "UPDATE")
            case "acquisition":
        """)

    def assert_runtime_credentials(self, runtime, payments, startup, assistant=None):
        pairwise = self.go_function_body(
            payments, "func independentRuntimeCredentials(credentials ...[]byte) error")
        self.assertEqual(pairwise, go_tokens(RUNTIME_CREDENTIAL_BODY))
        loader = self.go_function_body(
            payments, "func loadPaymentCredential(cfg Config, identityToken, commerceToken []byte) ([]byte, error)")
        self.assertEqual(loader, go_tokens(PAYMENT_CREDENTIAL_BODY))
        assistant_file = assistant if assistant is not None else (ROOT / COMPOSITION_ASSISTANT).read_text()
        assistant_loader = self.go_function_body(
            assistant_file, "func loadAssistantCredential(cfg Config, identity, commerce, payment []byte) ([]byte, error)")
        self.assertEqual(assistant_loader, go_tokens(ASSISTANT_CREDENTIAL_BODY))
        body = self.go_function_body(
            runtime, "func New(ctx context.Context, cfg Config) (_ *Runtime, resultErr error)")
        guard = self.token_index(body, """
            if err := independentRuntimeCredentials(cfg.HostCredential, token, commerceToken); err != nil {
                return nil, err
            }
        """)
        payment_load = self.token_index(body, """
            if selected["payments"] {
                paymentToken, err = loadPaymentCredential(cfg, token, commerceToken)
                if err != nil {
                    return nil, err
                }
            }
        """)
        assistant_load = self.token_index(body, """
            assistantToken, err := loadAssistantCredential(cfg, token, commerceToken, paymentToken)
            if err != nil {
                return nil, err
            }
        """)
        payment_compose = self.token_index(body, """
            if selected["payments"] {
                if err := r.composePayments(ctx, cfg, paymentToken, token, commerceToken, assistantToken, supportCustomerToken, supportNoteKey); err != nil {
                    return nil, err
                }
            }
        """)
        self.assertLess(guard, payment_load)
        self.assertLess(payment_load, assistant_load)
        self.assertLess(assistant_load, payment_compose)
        store_block = self.token_index(body, 'if selected["store"] {')
        store_open = self.token_index(body, "openStorage(ctx, cfg.Store)")
        commerce_client = self.token_index(body, "r.CommerceClient, err = coreclient.New(cfg.Socket, commerceToken)")
        assistant_block = self.token_index(body, 'if selected["assistant"] {')
        assistant_open = self.token_index(body, "openStorageBundle(ctx, cfg.Assistant, assistantStorageBundle(cfg))")
        assistant_delegation = self.token_index(body, "r.composeAssistantDelegation(ctx, cfg, db, auth, assistantToken)")
        operations_block = self.token_index(body, 'if selected["operations"] {')

        self.assertLess(assistant_load, store_block)
        self.assertLess(assistant_load, store_open)
        self.assertLess(assistant_load, commerce_client)
        self.assertLess(assistant_load, assistant_block)
        self.assertLess(assistant_load, assistant_open)
        self.assertLess(assistant_load, assistant_delegation)
        self.assertLess(assistant_load, operations_block)
        compose = self.go_function_body(
            payments, "func (r *Runtime) composePayments(ctx context.Context, cfg Config, paymentToken []byte, otherCredentials ...[]byte) error")
        self.token_index(compose, """
            r.PaymentClient, err = coreclient.New(cfg.Socket, paymentToken)
            if err != nil { return err }
        """)
        host_loader = self.go_function_body(
            startup, "func readHostCredential(path string) ([]byte, error)")
        self.assertEqual(host_loader, go_tokens("return coreclient.ReadTokenFile(path)"))
        run = self.go_function_body(startup, "func Run() error")
        loaded = self.token_index(run, """
            credential, err := readHostCredential(cfg.Host.TokenFile)
            if err != nil { return err }
        """)
        composition = self.token_index(run, "runtime, err := composition.New(context.Background(), composition.Config{")
        # Validate fields inside this initializer rather than requiring them to
        # be first. New module config fields must not shadow the checked token.
        initializer = self.braced_tokens(run, composition + len(go_tokens(
            "runtime, err := composition.New(context.Background(), composition.Config")))
        for field, value in (("Selected", "selected"), ("Socket", "socket"),
                             ("TokenFile", "tokenFile"), ("HostCredential", "credential")):
            self.token_index(initializer, f"{field}: {value},")
        self.assertLess(loaded, composition)

    def test_composition_keeps_distinct_module_pools_and_disabled_funds(self):
        source = (ROOT / "services/extensions/internal/composition/runtime.go").read_text()
        self.assert_composition_runtime(source)
        for required in COMPOSITION_RUNTIME_GUARDS:
            with self.subTest(required=required), self.assertRaises(AssertionError):
                self.assert_composition_runtime(source.replace(required, ""))
        for component in ("Community", "Acquisition"):
            for replacement in ("cfg.Store", "cfg.Support"):
                with self.subTest(component=component, shared=replacement), self.assertRaises(AssertionError):
                    self.assert_composition_runtime(source.replace("cfg." + component + ")", replacement + ")"))
            opener = source.index("db, err := openStorage(ctx, cfg." + component + ")")
            close = source.index("r.close = append(r.close, db.Close)", opener)
            changed = source[:close] + source[close:].replace("r.close = append(r.close, db.Close)", "", 1)
            with self.subTest(component=component, missing_close=True), self.assertRaises(AssertionError):
                self.assert_composition_runtime(changed)

    def test_central_credentials_guard_keeps_all_loaded_roles_pairwise_distinct(self):
        runtime = (ROOT / "services/extensions/internal/composition/runtime.go").read_text()
        payments = (ROOT / COMPOSITION_PAYMENTS).read_text()
        startup = (ROOT / "services/extensions/internal/app/run.go").read_text()
        assistant = (ROOT / COMPOSITION_ASSISTANT).read_text()
        self.assert_runtime_credentials(runtime, payments, startup, assistant)
        for required in ("credentials ...[]byte", "for i, token := range credentials",
                         "len(token) == 0", "credentials[i+1:]", "len(other) != 0",
                         "bytes.Equal(token, other)",
                         'errors.New("host and core RPC credentials must be independent")'):
            with self.subTest(removed=required), self.assertRaises(AssertionError):
                self.assert_runtime_credentials(runtime, payments.replace(required, ""), startup, assistant)
        # Each of the six possible role aliases must invalidate the exact
        # loaded-byte call. Empty optional roles are handled by the real helper.
        roles = ["cfg.HostCredential", "identityToken", "commerceToken", "paymentToken"]
        original = "independentRuntimeCredentials(" + ", ".join(roles) + ")"
        for i in range(len(roles)):
            for j in range(i + 1, len(roles)):
                aliased = list(roles)
                aliased[j] = aliased[i]
                changed = "independentRuntimeCredentials(" + ", ".join(aliased) + ")"
                with self.subTest(pair=(roles[i], roles[j])), self.assertRaises(AssertionError):
                    self.assert_runtime_credentials(runtime, payments.replace(original, changed), startup, assistant)
        as_roles = ["cfg.HostCredential", "identity", "commerce", "payment", "token"]
        as_original = "independentRuntimeCredentials(" + ", ".join(as_roles) + ")"
        for i in range(len(as_roles)):
            for j in range(i + 1, len(as_roles)):
                aliased = list(as_roles)
                aliased[j] = aliased[i]
                changed = "independentRuntimeCredentials(" + ", ".join(aliased) + ")"
                with self.subTest(as_pair=(as_roles[i], as_roles[j])), self.assertRaises(AssertionError):
                    self.assert_runtime_credentials(runtime, payments, startup, assistant.replace(as_original, changed))

    def test_actual_startup_uses_checked_credentials_before_financial_dependencies(self):
        runtime = (ROOT / "services/extensions/internal/composition/runtime.go").read_text()
        payments = (ROOT / COMPOSITION_PAYMENTS).read_text()
        startup = (ROOT / "services/extensions/internal/app/run.go").read_text()
        self.assert_runtime_credentials(runtime, payments, startup)
        guard = "independentRuntimeCredentials(cfg.HostCredential, token, commerceToken)"
        for changed in (runtime.replace(guard, "independentRuntimeCredentials(nil, token, commerceToken)"),
                        runtime.replace(guard, "nil /* " + guard + " */"),
                        runtime.replace(guard, guard + "; false"),
                        runtime.replace("loadPaymentCredential(cfg, token, commerceToken)",
                                        "loadPaymentCredential(cfg, token, token)"),
                        runtime.replace("r.composePayments(ctx, cfg, paymentToken, token, commerceToken, assistantToken, supportCustomerToken, supportNoteKey)",
                                        "r.composePayments(ctx, cfg, token, token, commerceToken, assistantToken, supportCustomerToken, supportNoteKey)"),
                        runtime.replace("loadAssistantCredential(cfg, token, commerceToken, paymentToken)",
                                        "loadAssistantCredential(cfg, token, token, paymentToken)"),
                        runtime.replace("loadAssistantCredential(cfg, token, commerceToken, paymentToken)",
                                        "loadAssistantCredential(cfg, token, commerceToken, token)")):
            with self.subTest(runtime=changed), self.assertRaises(AssertionError):
                self.assert_runtime_credentials(changed, payments, startup)
        # Store assembly moved before assistant credential validation (counterexample from verify.py)
        marker = '\tif selected["store"] {'
        start = runtime.index(marker)
        brace = runtime.index('{', start)
        depth = 0
        end = None
        for match in GO_TOKEN.finditer(runtime, brace):
            token = match.group(0)
            if token == '{': depth += 1
            elif token == '}':
                depth -= 1
                if depth == 0: end = match.end(); break
        self.assertIsNotNone(end)
        block = runtime[start:end]
        shortened = runtime[:start] + runtime[end:]
        pos = shortened.index('\tassistantToken, err := loadAssistantCredential(')
        store_before_assistant = shortened[:pos] + block + '\n' + shortened[pos:]
        with self.subTest(case="store_before_assistant"), self.assertRaises(AssertionError):
            self.assert_runtime_credentials(store_before_assistant, payments, startup)
        for required in ("coreclient.ReadTokenFile(cfg.PaymentTokenFile)",
                         "len(cfg.HostCredential) < 32", "return paymentToken, nil"):
            with self.subTest(removed=required), self.assertRaises(AssertionError):
                self.assert_runtime_credentials(runtime, payments.replace(required, ""), startup)
        with self.assertRaises(AssertionError):
            self.assert_runtime_credentials(runtime, payments.replace(
                "coreclient.New(cfg.Socket, paymentToken)", "coreclient.New(cfg.Socket, identityToken)"), startup)
        for old, new in (("HostCredential: credential", "HostCredential: nil"),
                         ("credential, err := readHostCredential(cfg.Host.TokenFile)",
                          "credential, err := []byte(nil), error(nil)"),
                         ("return coreclient.ReadTokenFile(path)", "return nil, nil")):
            with self.subTest(startup=old), self.assertRaises(AssertionError):
                self.assert_runtime_credentials(runtime, payments, startup.replace(old, new))

    def test_operations_adapters_borrow_and_keep_schema_boundaries(self):
        paths = (OPERATIONS_POSTGRES, OPERATIONS_RELEASES, OPERATIONS_INJECTION, OPERATIONS_RSS)
        for path in paths:
            source = (ROOT / path).read_text()
            self.assert_extension_storage(path, source)
            for forbidden in ("core_history.sources", "core_history.log_records",
                              "core_identity.users", "core_billing.balance_state", "core_events.outbox",
                              "lmm_store.orders", "lmm_support.messages", "lmm_assistant.records",
                              "lmm_promotions.records", "payment_orders"):
                with self.subTest(path=path, table=forbidden), self.assertRaises(AssertionError):
                    self.assert_extension_storage(path, source + "\nSELECT * FROM " + forbidden)
            for opener in ('sql.Open("postgres", dsn)', 'sql.OpenDB(connector)',
                           'sql . Open ("postgres", dsn)'):
                with self.subTest(path=path, opener=opener), self.assertRaises(AssertionError):
                    self.assert_extension_storage(path, source + "\n" + opener)
            aliased = source.replace('"database/sql"', 'dbsql "database/sql"').replace("sql.", "dbsql.")
            aliased += '\nfunc unapproved() { _, _ = dbsql.Open("postgres", "synthetic") }'
            with self.subTest(path=path, alias=True), self.assertRaises(AssertionError):
                self.assert_extension_storage(path, aliased)
            for method in ("Open", "OpenDB"):
                with self.subTest(path=path, method_value=method), self.assertRaises(AssertionError):
                    self.assert_extension_storage(path, source + "\nvar unapproved = sql." + method)
        releases = (ROOT / OPERATIONS_RELEASES).read_text()
        for added in ("func (s *Releases) Close() error { return s.db.Close() }",
                      "func (s *Releases) closeBorrowed() error { borrowed := s.db; return borrowed.Close() }"):
            with self.subTest(releases=added), self.assertRaises(AssertionError):
                self.assert_extension_storage(OPERATIONS_RELEASES, releases + "\n" + added)
        injection = (ROOT / OPERATIONS_INJECTION).read_text()
        for forbidden in ("lmm_operations.publications", "lmm_operations.reads"):
            with self.subTest(injection=forbidden), self.assertRaises(AssertionError):
                self.assert_extension_storage(OPERATIONS_INJECTION,
                                              injection + "\nSELECT * FROM " + forbidden)
        for old, new in (("Releases *sql.DB", "Releases *DatabaseConfig"),
                         ("NewPostgres(ctx, c.Releases)", "Open(ctx, *c.Releases)"),
                         ("return s, nil", "c.Releases.Close(); return s, nil")):
            with self.subTest(injection=old), self.assertRaises(AssertionError):
                self.assert_extension_storage(OPERATIONS_INJECTION, injection.replace(old, new))
        for added in ("func extra(c Config) error { return c.Releases.Close() }",
                      'func extra(c Config) { c.Releases.QueryContext(context.Background(), "SELECT 1") }',
                      "func extra(c Config) { borrowed := c.Releases; borrowed.Close() }",
                      "func (s *Service) Close() error { return nil }"):
            with self.subTest(injection_extra=added), self.assertRaises(AssertionError):
                self.assert_extension_storage(OPERATIONS_INJECTION, injection + "\n" + added)
        for path in ("services/extensions/internal/modules/store/postgres.go",
                     "services/extensions/internal/modules/support/postgres.go",
                     "services/extensions/internal/modules/assistant/postgres.go",
                     "services/extensions/internal/modules/promotions/postgres.go",
                     "services/extensions/internal/modules/payments/postgres.go"):
            source = (ROOT / path).read_text()
            with self.subTest(other_module=path), self.assertRaises(AssertionError):
                self.assert_extension_storage(path, source + "\nSELECT * FROM lmm_operations.publications")

    def test_rss_owner_upgrade_and_runtime_cas_remain_separate(self):
        for path, mutations in (
            (OFFLINE_OPERATIONS_CLI, (("f.NArg() != 0", "false"),
                ("session_user<>$3", "true"), ("NOT rolcanlogin", "true"),
                ("defer tx.Rollback()", ""), ("return ErrCommitUnknown", "return nil"))),
            (OFFLINE_RSS_UPGRADE, (("NOT rolcanlogin", "true"),
                ("actual != installed", "false"), ("if recoverOnly {", "if false {"),
                ("operation_id=$1", "true"), ("installed_by=session_user", "true"),
                ("GRANT SELECT,UPDATE ON lmm_operations.rss_config TO ", "GRANT ALL ON lmm_operations.rss_config TO "))),
            (OPERATIONS_RSS, (("WHERE singleton AND revision=$3", "WHERE singleton"),
                ("SELECT revision,feeds FROM lmm_operations.rss_config", "SELECT revision,feeds FROM lmm_operations.publications"))),
        ):
            source = (ROOT / path).read_text()
            self.assert_extension_storage(path, source)
            for old, new in mutations:
                changed = source.replace(old, new, 1)
                self.assertNotEqual(changed, source, (path, old))
                with self.subTest(path=path, mutation=old), self.assertRaises(AssertionError):
                    self.assert_extension_storage(path, changed)
        for path in ("services/extensions/internal/app/run.go", OPERATIONS_RSS,
                     "services/extensions/internal/webcompat/rss.go"):
            source = (ROOT / path).read_text()
            for call in ("operations.UpgradeRSS(ctx, tx, role, id, false)",
                         "operations.SealInstalledSchema(ctx, tx)",
                         "freshinstall.RunOperationsUpgradeCLI(ctx, args, out)",
                         'exec.Command("extensions", "upgrade-operations-db")'):
                with self.subTest(path=path, call=call), self.assertRaises(AssertionError):
                    self.assert_extension_storage(path, source + "\nfunc unapproved() { " + call + " }")
    def test_operations_pool_is_guarded_and_owned_by_composition(self):
        runtime = (ROOT / "services/extensions/internal/composition/runtime.go").read_text()
        storage = (ROOT / COMPOSITION_STORAGE).read_text()
        self.assert_composition_runtime(runtime)
        self.assert_extension_storage(COMPOSITION_STORAGE, storage)
        for old, new in (("openStorage(ctx, cfg.Operations)", "storageURL(cfg.Operations)"),
                         ("if resultErr != nil", "if false"),
                         ("i := len(r.close) - 1", "i := 0"),
                         ("config.Releases = db", "config.Releases = nil"),
                         ("config.Releases = db", "db.Close(); config.Releases = db"),
                         ("r.Operations, err = operations.New(ctx, config)",
                          "r.Operations, err = operations.New(ctx, operations.Config{})")):
            with self.subTest(runtime=old), self.assertRaises(AssertionError):
                self.assert_composition_runtime(runtime.replace(old, new))
        for required in ('add("lmm_operations.contract", "SELECT")',
                         'add("lmm_operations.publications", "SELECT", "INSERT")',
                         'add("lmm_operations.reads", "SELECT", "INSERT")',
                         "has_sequence_privilege(current_user,to_regclass('lmm_operations.publication_id'),$1)",
                         "has_database_privilege(current_user,oid,'CONNECT')",
                         "pg_has_role(current_user,oid,'SET')"):
            with self.subTest(storage=required), self.assertRaises(AssertionError):
                self.assert_extension_storage(COMPOSITION_STORAGE, storage.replace(required, ""))
        for old, new in (('[]string{"USAGE", "SELECT"}', '[]string{"SELECT"}'),
                         ('add("lmm_operations.reads", "SELECT", "INSERT")',
                          'add("lmm_operations.reads", "SELECT", "INSERT", "UPDATE")')):
            with self.subTest(storage=old), self.assertRaises(AssertionError):
                self.assert_extension_storage(COMPOSITION_STORAGE, storage.replace(old, new))

    def test_operations_pq_is_only_offline_identifier_quoting(self):
        source = (ROOT / OPERATIONS_POSTGRES).read_text()
        self.assert_extension_storage(OPERATIONS_POSTGRES, source)
        changes = (
            source + "\nvar unapproved = pq.NewConnector(dsn)",
            source + '\nvar unapproved = pq.QuoteIdentifier("runtime")',
            source.replace('"github.com/lib/pq"', '_ "github.com/lib/pq"'),
            source.replace('"github.com/lib/pq"', '. "github.com/lib/pq"'),
            source.replace('"github.com/lib/pq"', 'custom "github.com/lib/pq"'),
            source.replace("pq.QuoteIdentifier(runtimeRole)", "runtimeRole"),
            source.replace("pq.QuoteIdentifier(database)", "database"),
            source.replace("if err := Check(ctx, db); err != nil {", "if err := error(nil); err != nil {"),
            source.replace("return &Releases{db}, nil", "db.Close(); return &Releases{db}, nil"),
            source.replace("return &Releases{db}, nil", "Initialize(ctx, db, role); return &Releases{db}, nil"),
        )
        for index, changed in enumerate(changes):
            with self.subTest(change=index), self.assertRaises(AssertionError):
                self.assert_extension_storage(OPERATIONS_POSTGRES, changed)
        for required in OPERATIONS_CHECK_GUARDS:
            with self.subTest(check=required), self.assertRaises(AssertionError):
                self.assert_extension_storage(OPERATIONS_POSTGRES, source.replace(required, ""))

    def test_operations_rows_close_cannot_alias_the_borrowed_pool(self):
        source = (ROOT / OPERATIONS_RELEASES).read_text()
        self.assert_extension_storage(OPERATIONS_RELEASES, source)
        signature = "func (s *Releases) List(ctx context.Context, limit int) ([]ReleaseNote, error)"
        start = source.index(signature)
        end = source.index("\n}\n", start) + len("\n}")
        original = source[start:end]
        mutants = {
            "direct_alias": signature + " {\nrows := s.db\ndefer rows.Close()\nreturn []ReleaseNote{}, nil\n}",
            "nested_shadow": original.replace("defer rows.Close()",
                "{ rows := s.db; defer rows.Close() }"),
            "method_value": signature + " {\nrows := s.db\ncloseBorrowed := rows.Close\ndefer closeBorrowed()\nreturn []ReleaseNote{}, nil\n}",
        }
        for name, mutant in mutants.items():
            with self.subTest(alias=name), self.assertRaises(AssertionError):
                self.assert_extension_storage(OPERATIONS_RELEASES, source[:start] + mutant + source[end:])

    def test_extension_storage_guard_rejects_core_and_cross_module_access(self):
        path = "services/extensions/internal/modules/store/postgres.go"
        source = (ROOT / path).read_text()
        self.assert_extension_storage(path, source)
        for forbidden in ("core_history.sources", "core_history.log_records",
                          "core_identity.users", "core_billing.balance_state",
                          "core_events.outbox", "lmm_support.messages", "lmm_assistant.records",
                          "lmm_promotions.records", "payment_orders",
                          "sql.Open(", "sql.OpenDB("):
            with self.subTest(forbidden=forbidden):
                with self.assertRaises(AssertionError):
                    self.assert_extension_storage(path, source + "\nSELECT * FROM " + forbidden)
        with self.assertRaises(AssertionError):
            self.assert_extension_storage("services/extensions/internal/app/unapproved.go", source)

    def test_assistant_promotions_and_payment_storage_remain_narrow(self):
        for module, own in (("assistant", "lmm_assistant."), ("promotions", "lmm_promotions."),
                            ("payments", "payment_")):
            path = f"services/extensions/internal/modules/{module}/postgres.go"
            source = (ROOT / path).read_text()
            self.assert_extension_storage(path, source)
            for forbidden in ("core_history.sources", "core_history.log_records",
                              "core_identity.users", "core_billing.balance_state", "core_events.outbox",
                              "lmm_store.records", "lmm_support.records", "sql.Open(", "sql.OpenDB("):
                with self.subTest(module=module, forbidden=forbidden):
                    with self.assertRaises(AssertionError):
                        self.assert_extension_storage(path, source + "\nSELECT * FROM " + forbidden)
            for other in ("lmm_assistant.", "lmm_promotions.", "payment_"):
                if other != own:
                    with self.assertRaises(AssertionError):
                        self.assert_extension_storage(path, source + "\nSELECT * FROM " + other + "records")

    def test_borrowed_pool_adapters_cannot_access_core_without_sql_import(self):
        path = "services/extensions/internal/modules/store/money_recovery_postgres.go"
        source = (ROOT / path).read_text()
        self.assertNotIn('"database/sql"', source)
        # A future borrowed adapter must register before accessing even its own
        # schema. Core access stays forbidden both before and after registration.
        for adapter in (path, "services/extensions/internal/modules/store/future_recovery.go"):
            if adapter == path:
                self.assert_extension_storage(adapter, source)
            else:
                with self.assertRaises(AssertionError):
                    self.assert_extension_storage(adapter, source)
            for table in ("core_history.sources", "core_history.log_records", "core_billing.ledger_entries"):
                method = "\nfunc (p *Postgres) boundaryProbe(ctx context.Context) error {\n" + \
                    "_, err := p.db.ExecContext(ctx, `SELECT * FROM " + table + "`); return err\n}\n"
                with self.subTest(adapter=adapter, table=table), self.assertRaises(AssertionError):
                    self.assert_extension_storage(adapter, source + method)

    def test_new_extension_sql_registrations_are_exact_and_borrowed(self):
        direct = {
            "store/promotions.go": "lmm_store.",
            "store/purchase_limits.go": "lmm_store.",
            "toolmarket/postgres.go": "lmm_toolmarket.",
            "wallet/postgres.go": "extension_wallet.",
            "bounties/postgres.go": "lmm_bounties.",
            "assistant/agent_postgres.go": "lmm_assistant.",
            "assistant/personalization/service.go": "lmm_assistant.",
            "assistant_services/service.go": "lmm_assistant.",
            "assistant_services/memory.go": "lmm_assistant.",
            "assistant_services/confirmation.go": "lmm_assistant.",
            "assistant_services/issues.go": "lmm_assistant.",
            "assistant_services/platform_support.go": "lmm_assistant.",
            "assistant_services/search.go": "lmm_assistant.",
            "assistant_services/support_messages.go": "lmm_assistant.",
            "assistant_services/web_support.go": "lmm_assistant.",
        }
        direct.update({f"{module}/{name}": f"lmm_{module}."
                       for module, names in MODULE_STORAGE_FILES.items() for name in names})
        for leaf, own in direct.items():
            path = "services/extensions/internal/modules/" + leaf
            source = (ROOT / path).read_text()
            self.assert_extension_storage(path, source)
            for other in ("lmm_toolmarket.", "extension_wallet.", "lmm_bounties.", "lmm_assistant.",
                          "lmm_community.", "lmm_acquisition.", "lmm_unregistered.",
                          "core_directory_ads.", "core_referrals."):
                if other != own:
                    quoted = '"' + other[:-1] + '".'
                    for target in (other, quoted):
                        with self.subTest(path=path, target=target), self.assertRaises(AssertionError):
                            self.assert_extension_storage(path, source + "\nSELECT * FROM " + target + "records")
            for extra in ("db.Close()", "p.db.Close()", "borrowed := p.db; borrowed.Close()",
                          "borrowed := p.db; alias := borrowed; closePool := alias.Close; closePool()",
                          'var opener = sql.Open', 'sql.OpenDB(connector)'):
                with self.subTest(path=path, mutation=extra), self.assertRaises(AssertionError):
                    self.assert_extension_storage(path, source + "\nfunc unsafeBoundary() { " + extra + " }")
            future = path.replace(".go", "_unregistered.go")
            with self.subTest(future=future), self.assertRaises(AssertionError):
                self.assert_extension_storage(future, source)
        for leaf in ("toolmarket/store.go", "assistant/greeting.go", "assistant_services/actions.go", "assistant_services/dispatch.go"):
            path = "services/extensions/internal/modules/" + leaf
            source = (ROOT / path).read_text()
            self.assert_extension_storage(path, source)
            with self.subTest(type_only=path), self.assertRaises(AssertionError):
                self.assert_extension_storage(path, source + '\nfunc extra() { tx.ExecContext(ctx, "SELECT 1") }')

    def test_array_adapters_cannot_acquire_connections(self):
        for path in PQ_ARRAY_ADAPTERS:
            source = (ROOT / path).read_text()
            self.assert_extension_storage(path, source)
            for changed in (
                source.replace('"github.com/lib/pq"', '_ "github.com/lib/pq"'),
                source.replace('"github.com/lib/pq"', '. "github.com/lib/pq"'),
                source.replace('"github.com/lib/pq"', 'driver "github.com/lib/pq"').replace("pq.", "driver."),
                source.replace("pq.Array", "pq.NewConnector"),
                source + '\nvar opener = pq.NewConnector',
            ):
                with self.subTest(path=path, mutation=changed), self.assertRaises(AssertionError):
                    self.assert_extension_storage(path, changed)

    def test_every_core_schema_and_future_core_schema_remains_forbidden(self):
        declared = set()
        for path in (ROOT / "services/core/schema").glob("*.sql"):
            declared.update(name.lower() + "." for name in re.findall(r"(?i)CREATE\s+SCHEMA\s+(?:IF\s+NOT\s+EXISTS\s+)?(core_[a-z0-9_]+)", path.read_text()))
        self.assertTrue(declared <= set(CORE_SCHEMAS), sorted(declared-set(CORE_SCHEMAS)))
        for path in ("services/extensions/internal/modules/wallet/postgres.go",
                     "services/extensions/internal/modules/unregistered.go"):
            source = (ROOT / path).read_text() if (ROOT / path).exists() else "package unregistered\n"
            for schema in (*CORE_SCHEMAS, "core_future_unregistered."):
                for verb in ("FROM", "JOIN", "UPDATE", "INTO", "TRUNCATE TABLE", "REFERENCES", "ALTER TABLE", "CREATE TABLE", "DROP TABLE"):
                    for target in (schema, '"' + schema[:-1] + '".'):
                        with self.subTest(path=path, verb=verb, target=target), self.assertRaises(AssertionError):
                            self.assert_extension_storage(path, source + "\n" + verb + " " + target + "records")

    def test_resource_composition_cannot_gain_business_sql_or_extra_pool_close(self):
        path = "services/extensions/internal/composition/builtins.go"
        source = (ROOT / path).read_text()
        self.assert_extension_storage(path, source)
        for extra in ('db.QueryContext(ctx, "SELECT * FROM extension_wallet.transfer_links")',
                      'in.Database.QueryContext(ctx, "SELECT 1")',
                      "borrowed := in.Database; borrowed.Close()"):
            with self.subTest(mutation=extra), self.assertRaises(AssertionError):
                self.assert_extension_storage(path, source + "\nfunc extra() { " + extra + " }")
        for path in (COMPOSITION_STORAGE, "services/extensions/internal/composition/runtime.go",
                     COMPOSITION_PAYMENTS, "services/extensions/internal/composition/toolmarket.go"):
            source = (ROOT / path).read_text()
            for extra in ('var opener = sql.Open', 'func extra() { borrowed := db; borrowed.Close() }'):
                # Pool opening is registered only in storage; composition owns
                # pool shutdown solely through its existing resource stack.
                if path == COMPOSITION_STORAGE and "borrowed" in extra:
                    continue
                with self.subTest(path=path, mutation=extra), self.assertRaises(AssertionError):
                    self.assert_extension_storage(path, source + "\n" + extra)

    def test_offline_installer_is_closed_and_cannot_be_called_by_runtime(self):
        for path in (OFFLINE_MAIN, OFFLINE_INSTALLER, OFFLINE_CLI, OFFLINE_SNAPSHOT, OFFLINE_WALLET_CLI, OFFLINE_WALLET_UPGRADE):
            source = (ROOT / path).read_text()
            self.assert_extension_storage(path, source)
        installer = (ROOT / OFFLINE_INSTALLER).read_text()
        changes = (
            installer.replace('actual != c.ExpectedDatabase', 'false'),
            installer.replace('owner != c.OwnerRole', 'false'),
            installer.replace('session == c.RuntimeRole', 'false'),
            installer.replace('name.MatchString(c.OwnerRole)', 'true'),
            installer.replace('db != c.ExpectedDatabase', 'false'),
            installer.replace('len(v) != 1', 'false'),
            installer.replace('if e != nil || unsafe {', 'if false {'),
            installer.replace('tx.ExecContext(ctx, p.schema)', 'tx.ExecContext(ctx, userSQL)'),
            installer.replace('pq.QuoteIdentifier(c.OwnerRole)', 'c.OwnerRole'),
            installer.replace('defer tx.Rollback()', ''),
            installer.replace('nspowner=(SELECT oid FROM pg_roles WHERE rolname=$1)', 'false'),
            installer.replace("r.rolname NOT IN($2,'pg_database_owner')", 'false'),
            installer.replace('REVOKE ALL ON ALL TABLES IN SCHEMA "+q+" FROM "+role', 'SELECT 1'),
            installer.replace('REVOKE ALL ON ALL SEQUENCES IN SCHEMA "+q+" FROM "+role', 'SELECT 1'),
            installer.replace('REVOKE ALL ON ALL FUNCTIONS IN SCHEMA "+q+" FROM "+role', 'SELECT 1'),
            installer.replace('if e = tx.Commit(); e != nil {', 'if e = tx.Commit(); false {'),
            installer.replace('return Receipt{}, ErrCommitUnknown', 'return Receipt{}, nil'),
            installer + '\nfunc extra() { _, _ = sql.Open("postgres", "unapproved") }',
        )
        for index, changed in enumerate(changes):
            self.assertNotEqual(changed, installer, index)
            with self.subTest(installer=index), self.assertRaises(AssertionError):
                self.assert_extension_storage(OFFLINE_INSTALLER, changed)
        cli = (ROOT / OFFLINE_CLI).read_text()
        for changed in (cli.replace('f.NArg() != 0', 'false'),
                        cli + '\nfunc extra() { Install(context.Background(), Config{}) }',
                        cli.replace('"component"', '"sql-file"')):
            self.assertNotEqual(changed, cli)
            with self.subTest(cli=changed), self.assertRaises(AssertionError):
                self.assert_extension_storage(OFFLINE_CLI, changed)
        main = (ROOT / OFFLINE_MAIN).read_text()
        for changed in (main.replace('os.Args[1] == "install-db"', 'true'),
                        main.replace('\t\treturn\n', ''),
                        main.replace('os.Args[2:]', 'os.Args[1:]'),
                        main + '\nfunc init() { freshinstall.RunCLI(context.Background(), nil, os.Stdout) }'):
            self.assertNotEqual(changed, main)
            with self.subTest(main=changed), self.assertRaises(AssertionError):
                self.assert_extension_storage(OFFLINE_MAIN, changed)
        for path in ("services/extensions/internal/app/run.go",
                     "services/extensions/internal/composition/runtime.go",
                     "services/extensions/internal/modules/unapproved.go",
                     "services/extensions/internal/freshinstall/unapproved.go"):
            source = (ROOT / path).read_text() if (ROOT / path).exists() else "package unapproved\n"
            for extra in ('import installer "github.com/TokenNotIncluded/lmm-api/extensions/internal/freshinstall"',
                          'func extra() { freshinstall.Install(ctx, cfg) }',
                          'func extra() { exec.Command("extensions", "install-db").Run() }',
                          'var ownerFlag = "owner-url-file"',
                          'func extra() { toolmarket.InitPostgresSnapshot(ctx, db, tx, key) }',
                          'func extra() { _, _ = sql.Open("postgres", "owner") }'):
                with self.subTest(path=path, extra=extra), self.assertRaises(AssertionError):
                    self.assert_extension_storage(path, source + "\n" + extra)
        for path, replacements in (
            (OFFLINE_MAIN, (('os.Args[1] == "upgrade-wallet-db"', 'true'),
                            ('\t\treturn\n', ''))),
            (OFFLINE_WALLET_CLI, (('Recover(ctx, c)', 'Apply(ctx, c)'),)),
            (OFFLINE_WALLET_UPGRADE, (('c.OwnerRole == c.RuntimeRole', 'false'),
                ('!unix', 'false'), ('u.Path != "/"+c.Database', 'false'),
                ('c.Database != "lmm_toolmarket"', 'false'),
                ("pg_has_role($3,oid,'MEMBER')", 'false'),
                ("NOT has_database_privilege($2,current_database(),'CREATE,TEMP')", 'true'),
                ('defer tx.Rollback()', ''),
                ('REVOKE ALL ON extension_wallet.schema_upgrades FROM ', 'GRANT ALL ON extension_wallet.schema_upgrades TO '),
                ('return run(ctx, c, true)', 'return run(ctx, c, false)'))),
        ):
            source = (ROOT / path).read_text()
            for old, new in replacements:
                changed = source.replace(old, new, 1)
                self.assertNotEqual(changed, source, (path, old))
                with self.subTest(path=path, mutation=old), self.assertRaises(AssertionError):
                    self.assert_extension_storage(path, changed)
        for path in ("services/extensions/internal/app/run.go", "services/extensions/internal/composition/runtime.go",
                     "services/extensions/internal/walletupgrade/unapproved.go"):
            source = (ROOT / path).read_text() if (ROOT / path).exists() else "package unapproved\n"
            for extra in ('import "github.com/TokenNotIncluded/lmm-api/extensions/internal/walletupgrade"',
                          'func extra() { walletupgrade.RunCLI(ctx, nil, out) }',
                          'func extra() { walletupgrade.Apply(ctx, cfg); walletupgrade.Recover(ctx, cfg) }',
                          'var command = "upgrade-wallet-db"'):
                with self.subTest(path=path, extra=extra), self.assertRaises(AssertionError):
                    self.assert_extension_storage(path, source + "\n" + extra)
        snapshot = (ROOT / OFFLINE_SNAPSHOT).read_text()
        for changed in (snapshot.replace('tx.ExecContext(ctx,', 'db.ExecContext(ctx,'),
                        snapshot.replace('INSERT INTO', 'DELETE FROM'),
                        snapshot + '\nfunc extra() { db.Close() }'):
            self.assertNotEqual(changed, snapshot)
            with self.subTest(snapshot=changed), self.assertRaises(AssertionError):
                self.assert_extension_storage(OFFLINE_SNAPSHOT, changed)

    def test_explicit_storage_bundle_cannot_expand_default_or_lose_acl_guards(self):
        storage = (ROOT / COMPOSITION_STORAGE).read_text()
        policy = (ROOT / COMPOSITION_BUNDLE).read_text()
        market_path = "services/extensions/internal/composition/toolmarket.go"
        market = (ROOT / market_path).read_text()
        for changed in (
            storage.replace('return openStorageBundle(ctx, c, defaultStorageBundle)', 'return openStorageBundle(ctx, c, toolMarketBuiltinsBundle)'),
            storage.replace('policy, err := storagePolicyFor(c.Component, bundle)', 'policy, err := storagePolicyFor("toolmarket", toolMarketBuiltinsBundle)'),
            storage.replace("nspname LIKE 'core\\_%' ESCAPE '\\' OR", 'false OR'),
            storage.replace('NOT(nspname=ANY($1::text[]))', 'false'),
            storage.replace('($3 AND nspname NOT LIKE', '(false AND nspname NOT LIKE'),
            storage.replace('c.Component == "toolmarket"', 'false'),
            storage.replace('err != nil || !valid {', 'err != nil {', 1),
            storage.replace('has_any_column_privilege', 'has_table_privilege'),
            storage.replace('{limits.forbiddenColumns, "has_any_column_privilege", false}', '{policy.tables, "has_any_column_privilege", false}'),
            storage.replace('{limits.forbidden, "has_table_privilege", false}', '{limits.forbidden, "has_table_privilege", true}'),
            storage.replace('{limits.forbiddenColumns, "has_any_column_privilege", false}', '{limits.forbiddenColumns, "has_table_privilege", false}'),
            storage.replace('Scan(&valid); err != nil || !valid {\n\t\t\treturn nil, errors.New("extension storage grants differ from runtime contract")', 'Scan(&valid); !valid {\n\t\t\treturn nil, errors.New("extension storage grants differ from runtime contract")'),
            storage.replace('if extra || (required && !slices.Equal(actual, want))', 'if required && !slices.Equal(actual, want)'),
            storage.replace("has_column_privilege(current_user,attrelid,attnum,'UPDATE')", "false"),
            storage.replace('err != nil || excessive || (!allowed', 'err != nil || (!allowed'),
            storage.replace('if err := checkAssistantAuthority(bounded, db); err != nil', 'if err := error(nil); err != nil'),
            storage.replace('d.datdba=r.oid', 'false'),
            storage.replace('n.nspowner=r.oid', 'false'),
            storage.replace('c.relowner=r.oid', 'false'),
            storage.replace('aclexplode(n.nspacl)', 'aclexplode(NULL::aclitem[])'),
            storage.replace('aclexplode(c.relacl)', 'aclexplode(NULL::aclitem[])'),
            storage.replace('aclexplode(c.attacl)', 'aclexplode(NULL::aclitem[])'),
            storage.replace('a.is_grantable', 'false'),
            storage + '\nfunc extra() { _ = pq.NewConnector("unapproved") }',
        ):
            self.assertNotEqual(changed, storage)
            with self.subTest(storage=changed), self.assertRaises(AssertionError):
                self.assert_extension_storage(COMPOSITION_STORAGE, changed)
        for changed in (
            policy.replace('if cfg.AssistantAgentRequired {', 'if true {'),
            policy.replace('if cfg.AssistantTokenFile != "" {', 'if false {'),
            policy.replace('return assistantAgentBundle', 'return defaultStorageBundle'),
            policy.replace('component == "assistant"', 'component == "store"'),
            policy.replace('component == "toolmarket"', 'true'),
            policy.replace('bundle == toolMarketBuiltinsBundle', 'true'),
            policy.replace('"lmm_bounties", "extension_wallet"', '"lmm_bounties", "extension_wallet", "core_wallet"'),
            policy.replace('"MAINTAIN"', '"SELECT"'),
            policy.replace('policy.forbiddenColumns =', 'policy.tables ='),
        ):
            self.assertNotEqual(changed, policy)
            with self.subTest(policy=changed), self.assertRaises(AssertionError):
                self.assert_extension_storage(COMPOSITION_BUNDLE, changed)
        for changed in (
            market.replace('if cfg.ToolMarketBuiltins.configured() {', 'if true {'),
            market.replace('bundle := defaultStorageBundle', 'bundle := toolMarketBuiltinsBundle'),
            market.replace('openStorageBundle(ctx, cfg.ToolMarket, bundle)', 'openStorageBundle(ctx, cfg.ToolMarket, toolMarketBuiltinsBundle)'),
            market + '\nfunc extra() { openStorageBundle(ctx, cfg, toolMarketBuiltinsBundle) }',
        ):
            self.assertNotEqual(changed, market)
            with self.subTest(market=changed), self.assertRaises(AssertionError):
                self.assert_extension_storage(market_path, changed)
        runtime_path = "services/extensions/internal/composition/runtime.go"
        runtime = (ROOT / runtime_path).read_text()
        for old, new in (('assistantStorageBundle(cfg)', 'assistantAgentBundle'),
                         ('assistantStorageBundle(cfg)', 'defaultStorageBundle'),
                         ('if selected["assistant"] {', 'if true {')):
            changed = runtime.replace(old, new, 1)
            self.assertNotEqual(changed, runtime, old)
            with self.subTest(runtime=old), self.assertRaises(AssertionError):
                self.assert_extension_storage(runtime_path, changed)
        changed = runtime + "\nfunc extra() { openStorageBundle(ctx, cfg, defaultStorageBundle) }"
        with self.assertRaises(AssertionError):
            self.assert_extension_storage(runtime_path, changed)
        for path in ("services/extensions/internal/app/unapproved.go", "services/extensions/internal/composition/unapproved.go"):
            for extra in ('openStorageBundle(ctx, cfg, storageBundle(1))', 'toolMarketBuiltinsBundle'):
                with self.subTest(path=path, extra=extra), self.assertRaises(AssertionError):
                    self.assert_extension_storage(path, "package unapproved\nfunc extra() { " + extra + " }")

    def assert_core_command_boundary(self, main):
        # Rust lifetimes are not Go rune literals. In particular 'static must
        # not swallow the following command table up to the next apostrophe.
        tokens = [t for t in re.findall(
            r'//[^\n]*|/\*[\s\S]*?\*/|"(?:\\[\s\S]|[^"\\])*"|[A-Za-z_]\w*|\d+|[^\s]', main)
            if not t.startswith(("//", "/*"))]
        for name, variant in (("init-db", "InitDb"), ("bootstrap-user", "BootstrapUser")):
            self.token_index(tokens, f'("{name}", Command::{variant})')
        self.token_index(tokens, "Command::Serve => None,")
        self.token_index(tokens, "Command::InitDb => Some(admin::Operation::InitDb),")
        self.token_index(tokens, "Command::BootstrapUser => Some(admin::Operation::bootstrap(io::stdin().lock())?),")
        self.token_index(tokens, """match operation {
            Some(operation) => runtime.block_on(admin::run(operation)),
            None => runtime.block_on(run_core()),
        }""")
        serve = main.split("async fn run_core()", 1)[1]
        for forbidden in ("core_schema::init", "admin::run", "Operation::InitDb", "init_database"):
            self.assertNotIn(forbidden, serve)

    def test_fresh_schema_is_explicit_and_relational(self):
        schema = (ROOT / "services/core/schema/identity.sql").read_text()
        for table in ("accounts", "users", "teams", "memberships", "credentials", "key_funding_rules"):
            self.assertIn("CREATE TABLE core_identity." + table, schema)
        self.assertNotIn("funding_policy JSONB", schema)
        self.assertNotIn("DROP ", schema)
        runtime = (ROOT / "services/core/src/identity.rs").read_text()
        self.assertNotIn("MIGRATOR", runtime)
        self.assertNotIn("sqlx::migrate!", runtime)
        admin = (ROOT / "services/core/src/admin.rs").read_text()
        self.assertIn("core_schema::init(&pool)", admin)
        self.assertIn("core_schema::check(&pool)", admin)
        self.assertNotIn('"migrate"', admin)
        main = (ROOT / "services/core/src/main.rs").read_text()
        self.assert_core_command_boundary(main)
        self.assertNotIn("init_database", main.split("async fn run_core()", 1)[1])

    def test_core_management_shares_the_single_core_executable(self):
        cargo = tomllib.loads((ROOT / "services/core/Cargo.toml").read_text())
        self.assertFalse(cargo["package"]["autobins"])
        self.assertEqual(cargo["bin"], [{"name": "lmm-core", "path": "src/main.rs"}])
        self.assertFalse((ROOT / "services/core/src/bin/lmm-core-admin.rs").exists())

    def test_core_has_no_extension_startup_dependency(self):
        cargo = tomllib.loads((ROOT / "services/core/Cargo.toml").read_text())
        self.assertEqual(cargo["lints"]["rust"]["unsafe_code"], "forbid")
        self.assertNotIn("migrate", cargo["dependencies"]["sqlx"]["features"])
        for name in cargo["dependencies"]:
            self.assertNotIn("extension", name)
        for name in ("main.rs", "http.rs"):
            text = (ROOT / "services/core/src" / name).read_text()
            # Core must not read an Extensions or deployment identity variable
            # at runtime. Release provenance belongs to the private preflight,
            # not to a Core -> Extensions startup dependency or public header.
            extension_tokens = set(re.findall(
                r"(?<![A-Za-z0-9_])LMM_EXTENSION[A-Za-z0-9_]*", text
            ))
            self.assertEqual(
                extension_tokens,
                set(),
                name + ": Core must not depend on an Extensions runtime or release env",
            )

    def test_compose_projects_are_separate_and_resource_bounded(self):
        core = (ROOT / "deployment/docker/compose.core.yml").read_text()
        extension = (ROOT / "deployment/docker/compose.extensions.yml").read_text()
        self.assertTrue(core.startswith("name: lmm-core\n"))
        self.assertTrue(extension.startswith("name: lmm-extensions\n"))
        for body, owned, forbidden in ((core, "core", "extensions"), (extension, "extensions", "core")):
            services = body.split("services:\n", 1)[1].split("\nnetworks:", 1)[0]
            self.assertRegex(services, rf"(?m)^  {owned}:$")
            self.assertNotRegex(services, rf"(?m)^  {forbidden}:$")
            for field in ("read_only: true", "mem_limit:", "pids_limit:", "127.0.0.1:"):
                self.assertIn(field, body)
            self.assertNotIn("depends_on:", body)
            self.assertNotIn("docker.sock", body)
            self.assertNotIn("privileged: true", body)
        self.assertIn("external: true", extension)
        self.assertNotIn("SQL_DSN", extension)
        self.assertNotIn("DATABASE_URL", extension)
        self.assertIn("COPY services/extensions/", (ROOT / "deployment/docker/extensions.Dockerfile").read_text())
        self.assertIn("COPY services/core/schema", (ROOT / "deployment/docker/core.Dockerfile").read_text())

    def test_development_and_qualification_use_only_new_entry_points(self):
        self.assertFalse((ROOT / ".env.example").exists())
        for component in ("core", "extensions"):
            sample = tomllib.loads((ROOT / f"config/{component}.example.toml").read_text())
            self.assertEqual(sample["version"], 1)
        for name in ("ci.yml", "server-release-qualification.yml"):
            body = (ROOT / ".github/workflows" / name).read_text()
            self.assertIn("uses: ./.github/workflows/core-protocol.yml", body)
            for retired in ("migrate --", "credit-balance-rebase", "./model", "./relaykit", "apps/extensions-go"):
                self.assertNotIn(retired, body)

    def test_no_workflow_calls_deleted_go_business_packages(self):
        retired = ("services/extensions/relaykit", "./oauthserver", "./oidcprovider",
                   "./internal/marketprovider", "./model", "./controller", "./router",
                   "./service", "./middleware")
        for path in (ROOT / ".github/workflows").glob("*.yml"):
            body = path.read_text()
            for package in retired:
                self.assertNotIn(package, body, str(path))
        self.assertFalse((ROOT / ".github/workflows/coweft-identity.yml").exists())
        for name, expected in (("lmm.yml", "working-directory: apps/cli"),
                               ("codewhale-lmm-provider.yml", "packages/codewhale-lmm-provider"),
                               ("wallet-red-packet-review.yml", "working-directory: apps/web"),
                               ("tool-market-provider-verify.yml", "working-directory: apps/web")):
            self.assertIn(expected, (ROOT / ".github/workflows" / name).read_text())

    def test_standalone_core_has_bounded_resources_and_no_go_runtime(self):
        ignore = (ROOT / ".dockerignore").read_text().splitlines()
        self.assertIn("experiments/", ignore)
        compose = (ROOT / "deployment/docker/compose.core-standalone.yml").read_text()
        configuration = tomllib.loads((ROOT / "deployment/docker/config/core-standalone.toml").read_text())
        self.assertEqual(configuration["server"]["worker_threads"], 1)
        self.assertEqual(configuration["server"]["max_inflight"], 16)
        self.assertEqual(configuration["database"]["max_connections"], 2)
        self.assertEqual(configuration["identity"]["password_slots"], 1)
        self.assertEqual(configuration["database"]["url_file"], "/run/secrets/core_database_url")
        self.assertIn("./config/core-standalone.toml", compose)
        self.assertNotIn("extensions:", compose)
        self.assertNotIn("core-db:", compose)
        self.assertIn("name: ${COMPOSE_PROJECT_NAME}-runtime", compose)
        core = (ROOT / "services/core/src/main.rs").read_text()
        self.assertIn("config::database_pool", core)
        configuration = (ROOT / "services/core/src/config.rs").read_text()
        self.assertIn("bounded_database_pool(url, database_connections()?)", configuration)
        self.assertIn("Lifecycle::with_capacity", core)
        self.assertIn("with_password_slots", core)
        self.assertNotIn("lmm_extensions", core)

    def assert_runtime_database_guards(self, guard):
        uncommented = re.sub(r'//[^\n]*', '', guard)
        uncommented = re.sub(r'/\*.*?\*/', '', uncommented, flags=re.DOTALL)
        for required in ("REPEATABLE READ READ ONLY", "rolsuper", "rolbypassrls",
                         "rolcreaterole", "rolcreatedb", "rolreplication",
                         "has_any_column_privilege", "MEMBER WITH ADMIN OPTION",
                         "pg_has_role(current_user,r.oid,'SET')",
                         "core_migration", "core_billing", "c.relname<>'account_policies'",
                         "open_ledger_wallet"):
            self.assertIn(required, uncommented)
        for mutation in ("GRANT ", "ALTER ROLE ", "CREATE TABLE ", "INSERT INTO ",
                         "UPDATE core_", "DELETE FROM "):
            self.assertNotIn(mutation, uncommented)

        m = re.search(r'let\s+forbidden_write\s*:\s*bool\s*=\s*sqlx::query_scalar\(\s*r#"(.*?)"#\)', uncommented, re.DOTALL)
        self.assertIsNotNone(m, "forbidden_write query must be present in runtime_database.rs")
        sql = m.group(1)
        sql = re.sub(r'--[^\n]*', '', sql)
        sql = re.sub(r'/\*.*?\*/', '', sql, flags=re.DOTALL)
        norm_sql = " ".join(sql.split())

        billing_rule = re.search(
            r"\(\s*n\.nspname\s*=\s*'core_billing'\s+AND\s+c\.relname\s*<>\s*'account_policies'\s*\)",
            norm_sql
        )
        self.assertIsNotNone(billing_rule, "core_billing write protection rule must be active")
        start_pos = billing_rule.start()
        prefix = norm_sql[max(0, start_pos - 30):start_pos]
        self.assertFalse(re.search(r"\bFALSE\s+AND\b|\b0\s*=\s*1\s+AND\b", prefix, re.IGNORECASE), "billing rule must not be disabled with FALSE")
        suffix = norm_sql[billing_rule.end():min(len(norm_sql), billing_rule.end() + 30)]
        self.assertFalse(re.search(r"\bAND\s+FALSE\b|\bAND\s+0\s*=\s*1\b", suffix, re.IGNORECASE), "billing rule must not be disabled with FALSE")

        self.assertIn("has_table_privilege(c.oid,'INSERT,UPDATE,DELETE,TRUNCATE,TRIGGER,REFERENCES,MAINTAIN')", norm_sql)
        self.assertIn("has_any_column_privilege(c.oid,'INSERT,UPDATE,REFERENCES')", norm_sql)
        self.assertTrue(re.search(
            r"n\.nspname\s*=\s*'core_billing'\s+AND\s+c\.relname\s*=\s*'account_policies'\s+AND\s+has_table_privilege\(c\.oid,'DELETE,TRUNCATE,TRIGGER,REFERENCES,MAINTAIN'\)",
            norm_sql
        ), "account_policies DELETE/TRUNCATE protection must be present")
        self.assertTrue(re.search(
            r"n\.nspname\s+IN\s*\(\s*'core_billing'\s*,\s*'core_history'\s*\)\s+AND\s+CASE\s+WHEN\s+c\.relkind\s*=\s*'S'\s+THEN\s+has_sequence_privilege\(c\.oid,'USAGE,SELECT,UPDATE'\)",
            norm_sql
        ), "core_billing and core_history sequence privileges must both be checked")
        for fn in ('core_billing.import_opening_balance(jsonb)',
                   'core_billing.import_account_opening_counters(jsonb)',
                   'core_billing.import_legacy_escrow_batch(jsonb)'):
            self.assertIn(f"has_function_privilege('{fn}','EXECUTE')", norm_sql)

    def test_core_runtime_privileges_are_checked_without_writes(self):
        main = (ROOT / "services/core/src/main.rs").read_text()
        guard = (ROOT / "services/core/src/runtime_database.rs").read_text()
        self.assertIn("runtime_database::check", main)
        self.assert_runtime_database_guards(guard)
        # Counterexample 1: replacing billing write rule with FALSE (the exact verify.py mutant)
        mutated_false = guard.replace("(n.nspname='core_billing' AND c.relname<>'account_policies')",
                                      "(FALSE AND c.relname<>'account_policies')")
        with self.assertRaises(AssertionError):
            self.assert_runtime_database_guards(mutated_false)
        # Counterexample 2: commenting out billing write rule
        mutated_comment = guard.replace("(n.nspname='core_billing' AND c.relname<>'account_policies')",
                                        "/* (n.nspname='core_billing' AND c.relname<>'account_policies') */ true")
        with self.assertRaises(AssertionError):
            self.assert_runtime_database_guards(mutated_comment)
        # Counterexample 3: tampering billing schema name
        mutated_name = guard.replace("n.nspname='core_billing'", "n.nspname='tampered_billing'")
        with self.assertRaises(AssertionError):
            self.assert_runtime_database_guards(mutated_name)
        # Counterexample 4: allowing write to account_policies
        mutated_policy = guard.replace("c.relname<>'account_policies'", "true")
        with self.assertRaises(AssertionError):
            self.assert_runtime_database_guards(mutated_policy)
        # Counterexample 5: injecting an unsafe mutation command into guard
        mutated_insert = guard + "\nINSERT INTO core_billing.ledger_entries VALUES (1);"
        with self.assertRaises(AssertionError):
            self.assert_runtime_database_guards(mutated_insert)

        sequence_scope = "n.nspname IN ('core_billing','core_history')"
        self.assertEqual(guard.count(sequence_scope), 1)
        for retained in ("core_billing", "core_history"):
            changed = guard.replace(sequence_scope, "n.nspname='" + retained + "'")
            with self.subTest(retained_sequence_schema=retained), self.assertRaises(AssertionError):
                self.assert_runtime_database_guards(changed)

    def test_core_owns_the_funding_contract(self):
        cases = json.loads((ROOT / "contracts/core/v1/funding-cases.json").read_text())
        self.assertGreaterEqual(len(cases), 24)
        self.assertEqual(len({case["name"] for case in cases}), len(cases))
        self.assertIn("funding-cases.json", (ROOT / "services/core/src/funding.rs").read_text())


if __name__ == "__main__":
    unittest.main()
