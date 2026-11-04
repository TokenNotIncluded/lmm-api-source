#!/usr/bin/env python3
"""Offline identity-only import from an owned, immutable PostgreSQL snapshot.

Run through mise. Dry-run is the default. Receipts describe local Unix sockets;
this tool never connects over TCP, creates a schema, or imports funds, sessions,
API keys, OAuth links, passkeys, or second-factor secrets. Private artifacts must
stay below ~/.local/state/lmm-api. See --help for the explicit target receipt contract.

For a NOLOGIN target owner, connection.role is the effective owner, and the
explicit pair login_role/set_role names the actual LOGIN member and that same
owner. Startup PGOPTIONS performs the role switch; both session_user and the
effective owner are checked. Source readers cannot provide either field.

Replay is strict and only valid before console or other business imports. An
unconfirmed commit keeps its original migration ID and exact private materials.
A missing registry row does not prove rollback while the original session may
still run: wait for that session to end and fence with advisory lock 741803121
before resolving absence. Never switch IDs or automatically retry a failed SQL
transaction (including a serialization failure).
"""

from __future__ import annotations

import argparse
import base64
from collections import Counter
from datetime import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys
from typing import Any


BACKUP_ROOT = Path.home() / ".local/state/lmm-api"
# Bound the local client as well as connection setup. A write timeout does not
# establish rollback: retain the run ID/materials and resolve the original session.
PSQL_CLIENT_TIMEOUT_SECONDS = 600
READONLY_PSQL_TIMEOUT_OPTIONS = (
    " -c statement_timeout=300000 -c lock_timeout=5000"
    " -c idle_in_transaction_session_timeout=60000"
)
HEX = re.compile(r"[0-9a-f]{64}\Z")
IDENTIFIER = re.compile(r"[a-z_][a-z_0-9]*\Z")
BCRYPT = re.compile(r"\$2[aby]\$(0[4-9]|1[0-6])\$[./A-Za-z0-9]{53}\Z")
MIGRATION_ID = re.compile(r"[a-zA-Z0-9][a-zA-Z0-9_.-]{0,127}\Z")
# This deliberately accepts only the format the Rust verifier currently uses.
ARGON2 = re.compile(r"\$argon2id\$v=19\$m=19456,t=2,p=1\$([A-Za-z0-9+/]+)\$([A-Za-z0-9+/]+)\Z")
TABLES = ("accounts", "users", "user_profiles", "password_logins", "developer_access")
ACCOUNT_IMPORT_SQL = """  INSERT INTO core_identity.accounts(id,kind,service_level,active,created_at) OVERRIDING SYSTEM VALUE
   SELECT (r->>'account_id')::bigint,'personal',0,(r->>'active')::boolean,to_timestamp(coalesce((r->>'registered_at')::bigint,0)) FROM jsonb_array_elements(payload) r;"""
IDENTITY_VERSION = 6
# Bind this importer to the reviewed DDL, rather than trusting a caller's hash.
IDENTITY_FINGERPRINT = '28e38032e210565121fff19e095d36214ed2fc8f40016e3535c83929c0c60992'
IDENTITY_SCHEMA = Path(__file__).resolve().parents[1] / "services/core/schema/identity.sql"
IDENTITY_SCHEMA_RUST = Path(__file__).resolve().parents[1] / "services/core/src/identity/schema.rs"
ROOT = Path(__file__).resolve().parents[1]
CORE_SEED_CONTRACT = "fresh49-identity6-complete-seeds-v1"
# Reviewed fresh-49 assembly at 6416c4c, including alias/concat DDL and seeds.
# Cross-checked against the compiled schema-manifest. Byte changes require
# a new review, never a caller-provided replacement fingerprint. Order is DDL order.
# Routing additionally binds the reviewed fresh-install JSONB payload policy and
# JSON-preserving projection; this grants no existing-site upgrade authority.
REVIEWED_COMPONENTS = {
    'humangrant': ('include_str!("../schema/humangrant.sql")', '7d9437a2ee485bc7491c76e755ca481a7a722c4ea9ae7322fe32ac5f8f0090bc'),
    'identity': ('include_str!("../schema/identity.sql")', '28e38032e210565121fff19e095d36214ed2fc8f40016e3535c83929c0c60992'),
    'ledger': ('include_str!("../schema/ledger.sql")', 'ce973f6fee25904d4fcdc87895ec49e6c53e10c9fb3508df4d7d72ace16df77f'),
    'payments': ('include_str!("../schema/payments.sql")', '9579afddbe5aeb4deb67984e731bcdc6ca405dc619ceede8d92505ecc43f7d39'),
    'charging': ('include_str!("../schema/charging.sql")', '08f0a99e3cc7729627f782ec0341c31353008926b8ec8584c555a6c10dcb53de'),
    'routing': ('include_str!("../schema/routing.sql")', '9d91dc1796a39a32e1e4ce7c907eabf186687c7725230bd95af3b782fcf6a2a3'),
    'events': ('include_str!("../schema/events.sql")', '99b3f6e9daa36c97fd18d515c8654310945612c0d992297b816a0405c17d5f22'),
    'charging-ledger': ('include_str!("../schema/charging_ledger.sql")', 'a06370899332ece92cf54fe0fa24df70ea532480b09a19abf99a52a30a7d24de'),
    'migration': ('include_str!("../schema/migration.sql")', '4f6264d3edff1bd7a1f80289595a9e4dea596e325be7c6f4db568d03e738c78c'),
    'console': ('include_str!("../schema/console.sql")', '3857fec4b617c4b13e2a18f2a0a62386bdcf95ac1d4ce5d77e48215073e2c32e'),
    'model-gateway': ('include_str!("../schema/model_gateway.sql")', '3550655d1318c7b33e1877d6eb5b98eb47f045a9c4cfb801d23331fcccb00be4'),
    'commerce': ('include_str!("../schema/commerce.sql")', 'b7f9d07c4112623b34679b3043a1d315a4155ebe32688c2f878071a16f59b4c9'),
    'tool-market': ('include_str!("../schema/tool_market.sql")', '404a5a6f6a459bca2eeeaf19be312324933e81999ef160b4befa796668c4d11c'),
    'abuse': ('include_str!("../schema/abuse.sql")', 'b1c838d88f748c6bb3d885c4b39f112587bc8b7d38400c26ec43b3597110cec6'),
    'oauth-grants': ('include_str!("../schema/oauth_grants.sql")', 'e515ba3deca6d5b0b170fc5a7fa32cf40a42301b683b62e8bb80bc23683b16f3'),
    'history': ('include_str!("../schema/history.sql")', '9359e86f0f262e3f24bb9f8db70e32caab85cde515af6f2e5fdd1a66b4f83656'),
    'assistant-model': ('include_str!("../schema/assistant_model.sql")', 'c3960ce7ac307b6d4aecc0abccd0c97614220eeb7c20f960177bb53a3c2c3f6b'),
    'gift': ('include_str!("../schema/gift.sql")', '845cb755dd1ba92d9e5608f638d46367af3565bc3c5f383bb1aabc64929410d2'),
    'image': ('include_str!("../schema/image.sql")', '5ec9f1e4ea14093285de9bd34fecef5660b10e73ac2828de41f3c2c26f013d11'),
    'market-oauth': ('include_str!("../schema/market_oauth.sql")', '29819097b5470b4ecc8b1a7d7581d5abdc6b120df239edd6f4d373330086797e'),
    'tool-market-credit': ('include_str!("../schema/tool_market_credit.sql")', '59b861b2a316b79bf0842ab13c07d0570b07bf3b84945f4edf8957ace4cc733f'),
    'wallet': ('include_str!("../schema/wallet.sql")', 'd11528d84bd29c848daaf11fd58d92608955667b1ecfa1d42de92fba73de310e'),
    'bounties': ('include_str!("../schema/bounties.sql")', '965aa476d684fcfb0c72b6d58b7bb801d37135414f3c73f2a08c688e88143e06'),
    'assistant-tools': ('include_str!("../schema/assistant_tools.sql")', '9839e06cc0854c9efa60f30fef9c84a922dc18003b6317898df38a16c366aab4'),
    'trust-pricing': ('crate::billing::trust_pricing::SCHEMA', '93e0e16b04c312d9e1f27a0ae8dc25f16da4e7c044571cc14310b81de8cf1915'),
    'telegram-auth': ('crate::identity::TELEGRAM_AUTH_SCHEMA', 'd4734912cf68cdc8083356dc4301670cec27ec522e2c5ecaf2653a6a7d292510'),
    'assistant-agent': ('include_str!("../schema/assistant_agent.sql")', '2d4f29262bf9dec6fb10a82b2e8829a50b76030e40f5ebeed2a75eb0ea0ce1cf'),
    'site-finance': ('include_str!("../schema/site_statistics.sql")', '9bca87cc0cdc5ba96efb5c8bc7ef737684d74fcde257adba713a5951ebf63eb1'),
    'payment-gateways': ('include_str!("../schema/payment_gateways.sql")', '58c46cacccd631a14712915bb402c4c8bbdfc47b53b2c5fafa27a1aa32e91d97'),
    'request-limits': ('include_str!("../schema/request_limits.sql")', 'd8404a5a93dcfba21f64c9c2f1d9b2efee0ad3c381c6f6057d138a7a45825972'),
    'operations': ('include_str!("../schema/operations.sql")', '68bbf2e77a8751a3527ae6d14a4336fd394d12f780cc1b0a6aae8cf74709c5f2'),
    'coinbase-payments': ('include_str!("../schema/coinbase_payments.sql")', '222ea05eb4e7c5f24c5522b946edccde3cbee0d300a1da421dd5e62bafd1b3ee'),
    'channel-control': ('crate::channel_http::SCHEMA', '8f3caa43abf34475632047eaa2d59cc160e15ec90619379419d801b1b7b3726a'),
    'channel-vault': ('crate::channel_http::VAULT_SCHEMA', 'd5fe4fca492a12d6a853e635c0e8bc2d0479584744c5c350e6bded1f1ae4b6cd'),
    'passkeys': ('crate::identity::PASSKEY_SCHEMA', '26f610ad18bd62389e8b5304869d5e35bc84257605e8ecd54346a83824717ab6'),
    'account-lifecycle': ('crate::identity::ACCOUNT_LIFECYCLE_SCHEMA', '5ee797e0d99b7019622ac30b33efa0d124ad7dd44822e1d8088fdf2d160b6617'),
    'subscriptions': ('crate::subscriptions::SCHEMA', '9e1a631e70ed0fbdfafbc18229381f741fde18941a6708877768841945ee617d'),
    'promotions': ('crate::promotions::SCHEMA', '7f0caaf95f01242fd629006fc3089be43e66219b42992e5f0917ea6651ddab9b'),
    'notifications': ('include_str!("../schema/notifications.sql")', 'ab622593b37f4ed9bd053d6ffd8d3b75fdc9bb6296cb8f0031607f2f35acda21'),
    'account-links': ('crate::identity::ACCOUNT_LINKS_SCHEMA', '8feb800e3c71e634c1dd5d62d8cb6e2ed731790648028d55898946117307b179'),
    'completion-upgrade': ('include_str!("../schema/completion_upgrade.sql")', 'cbf657dbaff8ea48980f1cd8a7e2aa3d356fa11bda0c0b6a5c5e8e780833ef0a'),
    'account-appeals': ('crate::identity::ACCOUNT_APPEALS_SCHEMA', 'e9e14bc084de332bbef4fa858235bbb369f4a5df7e9b229a6a21bc0610252fb1'),
    'channel-accounting': ('crate::channel_http::ACCOUNTING_SCHEMA', '79e0d26947ceb515614f28f76c6de2cdb444943b9bad380b7d3e4915412c7040'),
    'channel-projection': ('concat!(include_str!("../schema/channel_projection.sql"),"\\n",include_str!("../schema/channel_openings.sql"))', '395f3a02d672e6b22eb854525f81b280155e32ad43c7175ec86a9c50a7611d4a'),
    'trust-policy-edit': ('concat!(include_str!("../schema/trust_policy_edit.sql"),"\\n",include_str!("../schema/trust_policy_edit_proof.sql"),"\\n",include_str!("../schema/trust_policy_edit_editor.sql"),"\\n",include_str!("../schema/trust_policy_upgrade.sql"))', '0d6588cb01bc829239b5186b4f20804c31d2c90d668f73506ca6b7485e20bc65'),
    'channel-observations': ('concat!(include_str!("../schema/channel_observations.sql"),"\\n",include_str!("../schema/channel_observations_upgrade.sql"))', '5d2784ba236898048958aa47558963671e234083db990e8ebd5f2b809c788dcb'),
    'navigation-options': ('include_str!("../schema/navigation_options.sql")', '4ef34139527fff615e8be1e34dd931b8d22ed61ec8497bcf15a7e150ea7c130c'),
    'red-packets': ('crate::promotions::red_packets::SCHEMA', '8df4f60aae98fc6555f07967d722d7c840c98353f4a5937e12e637a958171dde'),
    'legacy-history-compat': ('include_str!("../schema/legacy_history_compat.sql")', '02e778e7ee2d07a598844bf7d427e8408c22b256597841219113ea4b496507df'),
}
REVIEWED_SCHEMA_ALIASES = {
    'crate::billing::trust_pricing::SCHEMA': ('services/core/src/billing/trust_pricing.rs', 'SCHEMA', 'concat!(include_str!("../../schema/trust_pricing.sql"),"\\n",include_str!("../../schema/image_trust_extension.sql"))'),
    'crate::identity::TELEGRAM_AUTH_SCHEMA': ('services/core/src/identity/telegram.rs', 'SCHEMA', 'include_str!("../../schema/telegram_auth.sql")'),
    'crate::channel_http::SCHEMA': ('services/core/src/channel_http.rs', 'SCHEMA', 'include_str!("../schema/channel_control.sql")'),
    'crate::channel_http::VAULT_SCHEMA': ('services/core/src/channel_http.rs', 'VAULT_SCHEMA', 'include_str!("../schema/channel_vault.sql")'),
    'crate::channel_http::ACCOUNTING_SCHEMA': ('services/core/src/channel_http.rs', 'ACCOUNTING_SCHEMA', 'include_str!("../schema/channel_accounting.sql")'),
    'crate::identity::PASSKEY_SCHEMA': ('services/core/src/identity/browser/passkeys.rs', 'PASSKEY_SCHEMA', 'include_str!("../../../schema/passkeys.sql")'),
    'crate::identity::ACCOUNT_LIFECYCLE_SCHEMA': ('services/core/src/identity/browser/account_lifecycle.rs', 'ACCOUNT_LIFECYCLE_SCHEMA', 'include_str!("../../../schema/account_lifecycle.sql")'),
    'crate::identity::ACCOUNT_LINKS_SCHEMA': ('services/core/src/identity/browser/account_links.rs', 'ACCOUNT_LINKS_SCHEMA', 'include_str!("../../../schema/account_links.sql")'),
    'crate::identity::ACCOUNT_APPEALS_SCHEMA': ('services/core/src/identity/browser/account_appeals.rs', 'ACCOUNT_APPEALS_SCHEMA', 'include_str!("../../../schema/account_appeals.sql")'),
    'crate::subscriptions::SCHEMA': ('services/core/src/subscriptions.rs', 'SCHEMA', 'include_str!("../schema/subscriptions.sql")'),
    'crate::promotions::SCHEMA': ('services/core/src/promotions.rs', 'SCHEMA', 'include_str!("../schema/promotions.sql")'),
    'crate::promotions::red_packets::SCHEMA': ('services/core/src/promotions/red_packets.rs', 'SCHEMA', 'include_str!("../../schema/red_packets.sql")'),
}
CORE_COMPONENTS = frozenset(REVIEWED_COMPONENTS)
CATALOG_QUERY_SHA256 = 'b5a7bbfc27ebce87bd4e2ef7ba7f48d0bf21c56d572edebb6a84dcee61eca328'
OAUTH_CATALOG_QUERY_SHA256 = "39f6af7c6f19226a9daf794a5b897c630ca87670be3c1059f406fd8b4884cbca"
INSTALLER_CONTRACT_SHA256 = '8885db5795e5225baa8b0defee9f48c4c3a63029ed289107159b78336c759874'
REVIEWED_RUST_FILES = {
    'services/core/src/core_schema.rs': '222f4e27023c6c1bbb23cb6a437a74d389ae4e843032f5e04c23a7a031c5633a',
    'services/core/src/identity/schema.rs': 'eca66200583c53679c93633c78be9d8c4803977d7645b06612fa24bec1226612',
    'services/core/src/identity/grants.rs': 'f54840add7d997dc90e6a4f6f817f61b1fc1ff0acb6c8f5a29d6ae1f19f8b060',
    'services/core/src/identity.rs': 'b89b6b909ae4c513641f6ad9fad2e6dae62b32870188d9f84500ad6956111118',
    'services/core/src/identity/telegram.rs': '2dd9950fa5052230155ebc512dd12038adbeeab90bd7963bb8cbf3ca145bb5eb',
    'services/core/src/billing/trust_pricing.rs': 'c4c5f47d372fde4e5f7afd854a7a4691ef1b31ff74c1695d4242f0ab5133312f',
    'services/core/src/identity/browser.rs': '2a9943040778fcc95035e28ddac1fdbae9401ab107371528c3ef553e29611895',
    'services/core/src/channel_http.rs': 'b85ef9b4e9eb5cc80a8fa3ba32a8c0d097d0469ee57d52c42c3dde1530170155',
    'services/core/src/identity/browser/passkeys.rs': 'f4640a9bd6335a87df3d96702dec1a58d32350d204e361d3e74980722633710e',
    'services/core/src/identity/browser/account_lifecycle.rs': '30c479b52309df5adb6de229ee54eade90a55e7f3508bd9969ce7fff5db0172a',
    'services/core/src/identity/browser/account_links.rs': 'aea1a80c16ca9372d271c1fc5027be8a441a4dc85598844371e0a0b03e6e1a14',
    'services/core/src/identity/browser/account_appeals.rs': '1b5e24fcaa18cae2518989cd1ab9d10b896be18aed711d7125fbf859454de39d',
    'services/core/src/subscriptions.rs': '7eda58a0b9f82209ed517fe9ecb8f0e88280e6422845b86aaa5141e0a7e7034b',
    'services/core/src/promotions.rs': '677f2e5b195c6662905885fedf8ce60df4deb7b11a8f3dcae166dc00f4a172f6',
    'services/core/src/promotions/red_packets.rs': '4e7cdbe2c3a9f701f173631285e18c92d29350ebf7221beaff227f72446fd3bf',
}
DEVELOPER_POLICY = "legacy-explicit-override-before-manual-v1-paid-unproven"
# Identity import is deliberately narrower than a complete account cutover.
# Keep these invariants explicit instead of relying only on map_identity()'s
# current implementation: a hand-authored plan/manifest must not be able to
# claim that sessions, API keys, passkeys/OAuth links, or financial rows moved.
IDENTITY_ONLY_ZERO_FIELDS = ("sessions_imported", "api_keys_imported", "financial_rows_written")
# These fields are not emitted by the identity-only mapper today, but a
# hand-authored summary must not use them to imply that another credential
# family moved. If present for forward compatibility, they must remain zero.
IDENTITY_ONLY_FORBIDDEN_NONZERO_FIELDS = (
    "passkeys_imported", "oauth_bindings_imported", "oauth_server_tokens_imported",
    "mfa_factors_imported", "mfa_backup_codes_imported",
)
IDENTITY_ONLY_REQUIRED_GAPS = frozenset((
    "legacy_oauth_and_passkeys_archived_only",
    "financial_data_not_migrated",
))


class MappingError(Exception):
    """A safe machine-readable error. Never put a source value in this message."""


def require(condition: bool, code: str) -> None:
    if not condition:
        raise MappingError(code)


def canonical(value: Any) -> str:
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"), allow_nan=False)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def identity_contract(root: Path | None = None) -> dict:
    """Only the reviewed schema/version may generate or execute an import."""
    rust = IDENTITY_SCHEMA_RUST if root is None else root / "services/core/src/identity/schema.rs"
    schema = IDENTITY_SCHEMA if root is None else root / "services/core/schema/identity.sql"
    source = rust.read_text(encoding="utf-8")
    versions = re.findall(r"^const VERSION: i32 = ([0-9]+);$", source, re.MULTILINE)
    require(versions == [str(IDENTITY_VERSION)], "identity_source_version_mismatch")
    _reviewed_constant(source, "SCHEMA", 'include_str!("../../schema/identity.sql")')
    require(sha256_file(rust) == REVIEWED_RUST_FILES["services/core/src/identity/schema.rs"],
            "identity_rust_source_fingerprint_mismatch")
    fingerprint = sha256_file(schema)
    require(fingerprint == IDENTITY_FINGERPRINT, "identity_source_fingerprint_mismatch")
    return {"identity_version": IDENTITY_VERSION, "identity_fingerprint_sha256": fingerprint}


def _reviewed_constant(source: str, name: str, expression: str) -> None:
    declarations = re.findall(r"(?:pub(?:\(crate\))?\s+)?const\s+" + re.escape(name) +
                              r"\s*:\s*&str\s*=\s*(.*?);", source, re.S)
    def normalize(value):
        # Whitespace inside Rust strings is part of the compiled SQL/path.
        return re.sub(r'"(?:\\.|[^"\\])*"|\s+',
                      lambda match: match[0] if match[0].startswith('"') else "", value)
    require(len(declarations) == 1 and normalize(declarations[0]) == normalize(expression),
            "component_constant_not_reviewed")


# Distinct reviewed fresh-install profiles, not an upgrade path. The old49
# wallet included legacy transfer history; Git 0c12024 restored frozen wallet
# bytes and moved that history to component49. Never relabel an old49 receipt.
# 50: legacy verified payment proofs; 51: opening identity correction;
# 52: native affiliate claims; 53: explicit journal-kind/apply-entry revision;
# 54: pending referral settlement/clawback. These are fresh-install components,
# not permission to upgrade an existing49 site or replay registration credit.
CORE49_SEED_CONTRACT = CORE_SEED_CONTRACT
CORE54_SEED_CONTRACT = "fresh54-identity6-complete-seeds-v1"
CORE_SEED_CONTRACT = CORE54_SEED_CONTRACT
REVIEWED_COMPONENTS_54 = {
    **REVIEWED_COMPONENTS,
    "legacy-history-compat": ('include_str!("../schema/legacy_history_compat.sql")', 'ee05f13bb33859f6c9137500fd8023e41ee5f2db72c6e7788a8ad458e3e62744'),
    "wallet": ('include_str!("../schema/wallet.sql")', '0b75972ef1aaa6b8b09b2d5e84636ba90aa8affcb4c1c30fbb7de18a9a9b95ad'),
    'legacy-payments': ('include_str!("../schema/legacy_payments.sql")', '2541d8cab4781126a1eb16c986ed53a7b5c7a03bc22aecb05ece0b714108b744'),
    'legacy-payment-opening-identity': ('include_str!("../schema/legacy_payment_opening_identity.sql")', 'c2716051d9460f77acbb026a8fc224104d73f7eb4e54d153869919f45e2d92b8'),
    'affiliate-claims': ('include_str!("../schema/affiliate_claims.sql")', '9360480aa91db92d3bf66356b842f4944d0f868ea7771cc54e3417cb869e475a'),
    'referral-ledger-revision': ('include_str!("../schema/referral_ledger_revision.sql")', '4221800369a6ac582b367892b94d9b734a882f7d023de12a8e530bab29c7989e'),
    'legacy-referrals': ('include_str!("../schema/legacy_referrals.sql")', '8725422e057fadf00781c4557e60407773fd29893d85c35ff6045f0863574c4e'),
}
REVIEWED_RUST_FILES_54 = {'services/core/src/core_schema.rs': '21d7998841baace346fab98c9483a0fc53de9bdc4d2767534eaeda77923aa7b2',
 'services/core/src/identity/schema.rs': 'eca66200583c53679c93633c78be9d8c4803977d7645b06612fa24bec1226612',
 'services/core/src/identity/grants.rs': 'f54840add7d997dc90e6a4f6f817f61b1fc1ff0acb6c8f5a29d6ae1f19f8b060',
 'services/core/src/identity.rs': 'b89b6b909ae4c513641f6ad9fad2e6dae62b32870188d9f84500ad6956111118',
 'services/core/src/identity/telegram.rs': '2dd9950fa5052230155ebc512dd12038adbeeab90bd7963bb8cbf3ca145bb5eb',
 'services/core/src/billing/trust_pricing.rs': 'c4c5f47d372fde4e5f7afd854a7a4691ef1b31ff74c1695d4242f0ab5133312f',
 'services/core/src/identity/browser.rs': '2a9943040778fcc95035e28ddac1fdbae9401ab107371528c3ef553e29611895',
 'services/core/src/channel_http.rs': 'b85ef9b4e9eb5cc80a8fa3ba32a8c0d097d0469ee57d52c42c3dde1530170155',
 'services/core/src/identity/browser/passkeys.rs': 'f4640a9bd6335a87df3d96702dec1a58d32350d204e361d3e74980722633710e',
 'services/core/src/identity/browser/account_lifecycle.rs': '30c479b52309df5adb6de229ee54eade90a55e7f3508bd9969ce7fff5db0172a',
 'services/core/src/identity/browser/account_links.rs': 'aea1a80c16ca9372d271c1fc5027be8a441a4dc85598844371e0a0b03e6e1a14',
 'services/core/src/identity/browser/account_appeals.rs': '1b5e24fcaa18cae2518989cd1ab9d10b896be18aed711d7125fbf859454de39d',
 'services/core/src/subscriptions.rs': '7eda58a0b9f82209ed517fe9ecb8f0e88280e6422845b86aaa5141e0a7e7034b',
 'services/core/src/promotions.rs': '677f2e5b195c6662905885fedf8ce60df4deb7b11a8f3dcae166dc00f4a172f6',
 'services/core/src/promotions/red_packets.rs': '4e7cdbe2c3a9f701f173631285e18c92d29350ebf7221beaff227f72446fd3bf',
 'services/core/src/core_schema/manifest.rs': 'a48d29c77e8bd92652c69785b3a82e2ae1995c52adcc3296f89a91ae911df6a5',
 'services/core/src/core_schema/navigation_options.rs': 'cc63a7b6c26834e25a750a50eefe370ee005a1fe21b7260dd55f414ca4f62ca0',
 'services/core/src/core_schema/upgrade/fresh_target.json': '6efa195fe456a7ca30ffedc799431c51743d57c9f648e21fcf18c6de82963cb6'}
CORE_COMPONENTS = frozenset(REVIEWED_COMPONENTS_54)
CORE54_SCHEMA_REVISION = "ee8d1b4f6690cc6494aa0454a59f5445b4c83d3d"
CORE54_MANIFEST_SHA256 = "2632145eaf6cdf3cdbf7141e89831f25c8b4d4a8426e2d4ad95ad6408151e1eb"
CORE54_CATALOG_SHA256 = "3a7ba0ed308f2c4ce9f80e422d5308ab8353d8f21cfb5a63575fbabb678311fc"


def reviewed_profile(root: Path | None = None) -> str:
    """An exact reviewed assembly selects a profile; no arbitrary suffixes."""
    root = ROOT if root is None else Path(root)
    digest = sha256_file(root / "services/core/src/core_schema.rs")
    if digest == REVIEWED_RUST_FILES_54["services/core/src/core_schema.rs"]:
        return CORE54_SEED_CONTRACT
    require(digest == REVIEWED_RUST_FILES["services/core/src/core_schema.rs"],
            "component_rust_source_fingerprint_mismatch")
    return CORE49_SEED_CONTRACT


def reviewed_components(root: Path) -> dict:
    return REVIEWED_COMPONENTS_54 if reviewed_profile(root) == CORE54_SEED_CONTRACT else REVIEWED_COMPONENTS


def reviewed_files(root: Path) -> dict:
    return REVIEWED_RUST_FILES_54 if reviewed_profile(root) == CORE54_SEED_CONTRACT else REVIEWED_RUST_FILES


def _reviewed_rust(root: Path, relative: str) -> str:
    path = root / relative
    require(sha256_file(path) == reviewed_files(root)[relative], "component_rust_source_fingerprint_mismatch")
    return path.read_text(encoding="utf-8")


def _component_schema_paths(root: Path, expression: str, source_path: Path) -> list[Path]:
    paths = []
    for relative in re.findall(r'include_str!\("([^"\\]+)"\)', expression):
        path = (source_path.parent / relative).resolve()
        require(path.parent == (root / "services/core/schema").resolve(), "component_contract_invalid")
        paths.append(path)
    require(bool(paths), "component_expression_not_reviewed")
    return paths


def _component_bytes(root: Path, expression: str, source_path: Path) -> bytes:
    paths = iter(_component_schema_paths(root, expression, source_path))
    # Only pinned include expressions and explicit LF separators are accepted.
    chunks = []
    for match in re.finditer(r'include_str!\("[^"\\]+"\)|"\\n"', expression):
        chunks.append(next(paths).read_bytes() if match[0].startswith('include_str!') else b"\n")
    return b"".join(chunks)


def component_schemas(root: Path | None = None) -> dict[str, bytes]:
    """Resolve the complete pinned fresh assembly, never arbitrary Rust code."""
    root = ROOT if root is None else Path(root)
    path = root / "services/core/src/core_schema.rs"
    source = _reviewed_rust(root, "services/core/src/core_schema.rs")
    blocks = re.findall(r"const COMPONENTS:\s*&\[\(&str,\s*&str\)\]\s*=\s*&\[(.*?)\];", source, re.S)
    require(len(blocks) == 1, "component_contract_syntax_unknown")
    include = r'include_str!\("[^"\\]+"\)'
    concat = r'concat!\(\s*(?:(?:' + include + r'|"\\n")\s*,?\s*)+\)'
    entries = re.compile(r'\(\s*"([a-z][a-z-]*)"\s*,\s*(' + include + '|' + concat + r'|[a-zA-Z_][a-zA-Z_0-9:]*)\s*,?\s*\)')
    require(re.fullmatch(r"[\s,]*", entries.sub("", blocks[0])) is not None, "component_contract_syntax_unknown")
    parsed = entries.findall(blocks[0])
    names = [name for name, _ in parsed]
    require(len(names) == len(set(names)), "component_contract_duplicate")
    require(names == list(reviewed_components(root)), "component_contract_incomplete")
    components = {}
    for name, expression in parsed:
        expected_expression, expected_digest = reviewed_components(root)[name]
        expression = re.sub(r"\s+", "", expression)
        require(expression == expected_expression, "component_expression_not_reviewed")
        source_path = path
        if expression in REVIEWED_SCHEMA_ALIASES:
            relative, constant, expression = REVIEWED_SCHEMA_ALIASES[expression]
            alias_source = _reviewed_rust(root, relative)
            _reviewed_constant(alias_source, constant, expression)
            source_path = root / relative
            # Identity re-exports are pinned across both modules, not guessed.
            if name == "telegram-auth":
                identity = _reviewed_rust(root, "services/core/src/identity.rs")
                aliases = re.findall(r"pub(?:\(crate\))?\s+use\s+([^;]*?)\s+as\s+TELEGRAM_AUTH_SCHEMA\s*;", identity)
                require(aliases == ["telegram::SCHEMA"], "component_alias_not_reviewed")
        data = _component_bytes(root, expression, source_path)
        require(hashlib.sha256(data).hexdigest() == expected_digest, "component_source_fingerprint_mismatch")
        components[name] = data
    return components


def installer_metadata(root: Path | None = None) -> bytes:
    """Exact reviewed metadata DDL; callers may inspect, never auto-install it."""
    root = ROOT if root is None else Path(root)
    source = _reviewed_rust(root, "services/core/src/core_schema.rs")
    contracts = re.findall(r'^const CONTRACT: &str = "([^"\\]*)";$', source, re.M)
    require(len(contracts) == 1, "installer_contract_syntax_unknown")
    _reviewed_constant(source, "UPGRADE_METADATA", 'include_str!("../schema/core_upgrade.sql")')
    _reviewed_constant(source, "ADDITIVE_UPGRADE_METADATA", 'include_str!("../schema/component_upgrade.sql")')
    _reviewed_constant(source, "LEDGER_REVISION_METADATA", 'include_str!("../schema/ledger_revision.sql")')
    _reviewed_constant(source, "CONSOLE_REVISION_METADATA", 'include_str!("core_schema/upgrade/console_receipt.sql")')
    _reviewed_constant(source, "OFFLINE_REVISION_METADATA", 'include_str!("../schema/offline_upgrade.sql")')
    data = contracts[0].encode("utf-8") + b"".join((root / relative).read_bytes() for relative in (
        "services/core/schema/core_upgrade.sql", "services/core/schema/component_upgrade.sql",
        "services/core/schema/ledger_revision.sql", "services/core/src/core_schema/upgrade/console_receipt.sql",
        "services/core/schema/offline_upgrade.sql"))
    require(hashlib.sha256(data).hexdigest() == INSTALLER_CONTRACT_SHA256, "installer_source_fingerprint_mismatch")
    return data


def current_contract(root: Path | None = None) -> dict:
    """Bind every compiled component, identity version and catalog query."""
    root = ROOT if root is None else Path(root)
    expected = identity_contract(root)
    for path in reviewed_files(root):
        _reviewed_rust(root, path)
    components = {name: hashlib.sha256(data).hexdigest() for name, data in component_schemas(root).items()}
    source = _reviewed_rust(root, "services/core/src/core_schema.rs")
    catalogs = re.findall(r'const CATALOG:\s*&str\s*=\s*r#"(.*?)"#;', source, re.S)
    require(len(catalogs) == 1, "catalog_contract_syntax_unknown")
    require(hashlib.sha256(catalogs[0].encode()).hexdigest() == CATALOG_QUERY_SHA256, "catalog_source_fingerprint_mismatch")
    paths = set(reviewed_files(root)) | {
        "services/core/schema/core_upgrade.sql", "services/core/schema/component_upgrade.sql",
        "services/core/schema/ledger_revision.sql", "services/core/schema/offline_upgrade.sql",
        "services/core/src/core_schema/upgrade/console_receipt.sql"}
    for expression, _ in reviewed_components(root).values():
        source_path = root / "services/core/src/core_schema.rs"
        if expression in REVIEWED_SCHEMA_ALIASES:
            relative, _, expression = REVIEWED_SCHEMA_ALIASES[expression]
            source_path = root / relative
        paths.update(str(path.relative_to(root.resolve())) for path in _component_schema_paths(root, expression, source_path))
    contract = {**expected, "component_fingerprints": components,
            "catalog_query_sha256": CATALOG_QUERY_SHA256, "catalog_query": catalogs[0],
            "installer_contract_sha256": hashlib.sha256(installer_metadata(root)).hexdigest(),
            "oauth_catalog_query_sha256": hashlib.sha256(oauth_catalog_query(root).encode()).hexdigest(),
            "source_file_hashes": {path: sha256_file(root / path) for path in sorted(paths)}}
    if reviewed_profile(root) == CORE54_SEED_CONTRACT:
        # Match serde_json::to_vec(schema_manifest()) plus the CLI's final LF.
        # Both producing Rust and fresh_target.json are independently pinned.
        manifest = {"format_version": 1, "scope": "compiled-core-schema-contract",
            "components": [{"name": name, "raw_schema_sha256": digest} for name, digest in components.items()],
            "identity": {"version": expected["identity_version"], "fingerprint_sha256": expected["identity_fingerprint_sha256"]},
            "installer_contract_sql_sha256": contract["installer_contract_sha256"],
            "catalog": {"query_utf8": catalogs[0], "query_sha256": CATALOG_QUERY_SHA256,
                "query_hash_domain": "sha256-raw-utf8-v1",
                "fingerprint_hash_domain": "sha256-u64be-byte-length-prefixed-utf8-v1",
                "definition_order": "postgresql-c-collation"}}
        digest = hashlib.sha256((json.dumps(manifest, ensure_ascii=False, separators=(",", ":")) + "\n").encode()).hexdigest()
        pin = json.loads((root / "services/core/src/core_schema/upgrade/fresh_target.json").read_text())
        require(digest == CORE54_MANIFEST_SHA256 and pin == {
            "manifest_sha256": CORE54_MANIFEST_SHA256, "catalog_sha256": CORE54_CATALOG_SHA256},
            "compiled_manifest_contract_mismatch")
        contract.update(import_profile=CORE54_SEED_CONTRACT, source_schema_revision=CORE54_SCHEMA_REVISION,
                        compiled_manifest_sha256=digest,
                        fresh_catalog_sha256=CORE54_CATALOG_SHA256,
                        seed_query_sha256=hashlib.sha256(fresh_seed_hash_query().encode()).hexdigest())
    return contract


def catalog_hash_sql(contract: dict) -> str:
    # Match Rust's UTF-8 definitions, each prefixed with its 8-byte BE length.
    return "SELECT encode(sha256(coalesce(string_agg(int8send(octet_length(convert_to(definition,'UTF8'))::bigint)||convert_to(definition,'UTF8'),''::bytea ORDER BY definition COLLATE \"C\"),''::bytea)),'hex') FROM (" + contract["catalog_query"] + ") catalog_definitions"


def oauth_catalog_query(root: Path | None = None) -> str:
    root = ROOT if root is None else Path(root)
    source = _reviewed_rust(root, "services/core/src/identity/grants.rs")
    catalogs = re.findall(r'const CATALOG:\s*&str\s*=\s*r#"(.*?)"#;', source, re.S)
    require(len(catalogs) == 1, "oauth_catalog_contract_syntax_unknown")
    require('VALUES (1,$1,$2)' in source, "oauth_seed_version_unknown")
    require(hashlib.sha256(catalogs[0].encode()).hexdigest() == OAUTH_CATALOG_QUERY_SHA256, "oauth_catalog_contract_changed")
    return catalogs[0]


def oauth_seed_guard_sql(contract: dict) -> str:
    """Validate the installer-written OAuth receipt, including its actual catalog."""
    catalog = oauth_catalog_query()
    require(hashlib.sha256(catalog.encode()).hexdigest() == contract.get("oauth_catalog_query_sha256"),
            "oauth_catalog_contract_changed")
    # grants::catalog uses the same UTF-8, 8-byte length-prefixed digest as Core,
    # with its sole output column named d. The complete Core catalog is checked too.
    query = "SELECT d AS definition FROM (" + catalog + ") oauth_definitions"
    actual = catalog_hash_sql({"catalog_query": query})
    fingerprint = sql_literal(contract["component_fingerprints"]["oauth-grants"])
    return ("(SELECT count(*) FROM core_oauth.contract)<>1 OR NOT EXISTS("
            "SELECT 1 FROM core_oauth.contract WHERE singleton AND version=1 "
            "AND encode(ddl_fingerprint,'hex')=" + fingerprint +
            " AND encode(catalog_fingerprint,'hex')=(" + actual + "))")


DEFAULT_TRUST_POLICY = {
    "version": 1, "paid_activation_enabled": True, "decay_period_days": 90,
    "tiers": [
        {"level": 0, "min_paid_credits": 0, "discount": {"numerator": 1, "denominator": 1}},
        {"level": 1, "min_paid_credits": 500000, "discount": {"numerator": 1, "denominator": 1}},
        {"level": 2, "min_paid_credits": 5000000, "discount": {"numerator": 97, "denominator": 100}},
        {"level": 3, "min_paid_credits": 25000000, "discount": {"numerator": 94, "denominator": 100}},
        {"level": 4, "min_paid_credits": 100000000, "discount": {"numerator": 9, "denominator": 10}},
    ],
    "admin_discount": {"numerator": 9, "denominator": 10},
    "superadmin_discount": {"numerator": 9, "denominator": 10},
}


def utc_timestamp_text_sql(column: str) -> str:
    """Match PostgreSQL UTC JSON timestamp text without losing fractional digits.

    The caller supplies a reviewed SQL column expression. Converting to UTC
    before JSON serialization also makes standalone readbacks independent of
    session/database TimeZone, without changing settings inside a SELECT.
    """
    return "(to_jsonb(" + column + " AT TIME ZONE 'UTC') #>> '{}') || '+00:00'"


def trust_policy_seed_query() -> str:
    """Owner captures the exact UTC readback before approving a fresh import."""
    return ("SELECT to_jsonb(p)||jsonb_build_object('source_digest',encode(source_digest,'hex'),"
            "'native_since'," + utc_timestamp_text_sql("p.native_since") + ") "
            "FROM core_trust_pricing.policies p")


def validate_trust_policy_seed(seed: dict) -> None:
    fixed = {"singleton": True, "revision": 1, "spec": DEFAULT_TRUST_POLICY,
             "source_digest": hashlib.sha256(b"core-native-trust-policy-v1").hexdigest(),
             "native_user_floor": 1}
    require(isinstance(seed, dict) and set(seed) == set(fixed) | {"source_cutoff", "native_since"},
            "trust_seed_receipt_invalid")
    # Canonical JSON comparison distinguishes booleans from integers and covers
    # every nested policy field. No caller-chosen policy is a fresh seed.
    require(all(canonical(seed[key]) == canonical(value) for key, value in fixed.items()),
            "trust_seed_receipt_not_default")
    require(type(seed["source_cutoff"]) is int and 0 < seed["source_cutoff"] <= 253402300799
            and isinstance(seed["native_since"], str), "trust_seed_receipt_invalid")
    try:
        since = datetime.fromisoformat(seed["native_since"])
        valid = since.tzinfo is not None and since.timestamp() >= seed["source_cutoff"]
    except (ValueError, OverflowError):
        valid = False
    require(valid, "trust_seed_receipt_invalid")


def trust_policy_seed_guard_sql(contract: dict) -> str:
    seed = contract.get("trust_policy_seed")
    validate_trust_policy_seed(seed)
    return ("(SELECT count(*) FROM core_trust_pricing.policies)<>1 OR (" +
            trust_policy_seed_query() + ") IS DISTINCT FROM " + sql_literal(canonical(seed)) + "::jsonb")


UPGRADE_RECEIPT_TABLES = (
    "core_meta.schema_upgrades", "core_meta.component_upgrades", "core_meta.ledger_revisions",
    "core_meta.console_revisions", "core_meta.offline_upgrades", "core_meta.site_finance_upgrades",
    "core_meta.payment_gateway_upgrades", "core_meta.coinbase_payment_upgrades",
    "core_meta.completion_upgrades", "core_meta.account_appeal_upgrades",
    "core_meta.channel_accounting_upgrades", "core_meta.channel_projection_upgrades",
    "core_meta.trust_policy_upgrades", "core_meta.channel_observations_upgrades",
    "core_meta.navigation_options_upgrades",
)


def upgrade_receipts_guard_sql() -> str:
    return " OR ".join("EXISTS(SELECT 1 FROM " + table + ")" for table in UPGRADE_RECEIPT_TABLES)


def fresh_seed_hash_query(profile: str = CORE54_SEED_CONTRACT) -> str:
    """Bind all seed fields in UTC, including the physical sequence row fields."""
    relations = {
        "ledger_accounts": ("core_billing.ledger_accounts", "id"),
        "balance_state": ("core_billing.balance_state", "ledger_account_id"),
        "ledger_sequence": ("core_billing.ledger_accounts_id_seq", "last_value"),
        "event_capacity": ("core_events.capacity", "singleton"),
        "routing_release": ("public.routing_active_release", "singleton"),
        "abuse_settings": ("core_abuse.settings", "singleton"),
        "oauth_contract": ("core_oauth.contract", "singleton"),
        "human_grant_gate": ("core_humangrant.human_grant_gate", "id"),
        "gift_risk_key": ("core_gift.risk_key", "singleton"),
        "gift_policy": ("core_gift.policy", "singleton"),
        "credit_policy": ("core_tool_market.credit_policy", "singleton"),
        "trust_policy": ("core_trust_pricing.policies", "singleton"),
        "request_policy": ("core_request_limits.policy", "singleton"),
        "request_auto_policy": ("core_request_limits.auto_policy", "singleton"),
        "promotion_secret": ("core_promotions.secrets", "singleton"),
        "channel_projection_state": ("core_channels.projection_state", "singleton"),
    }
    require(profile in (CORE49_SEED_CONTRACT, CORE54_SEED_CONTRACT), "seed_profile_not_reviewed")
    if profile == CORE54_SEED_CONTRACT:
        # 50--54 add no seed rights: all twelve tables must remain empty.
        # Their whole rows and both new physical sequences are still hashed.
        relations.update({
            "legacy_bases": ("core_payments.legacy_bases", "id"),
            "legacy_proofs": ("core_payments.legacy_proofs", "id"),
            "provider_identifiers": ("core_payments.provider_identifiers", "provider,merchant,environment,kind,identifier"),
            "legacy_applications": ("core_payments.legacy_applications", "operation_id"),
            "affiliate_claims": ("core_wallet.affiliate_claims", "id"),
            "affiliate_claims_sequence": ("core_wallet.affiliate_claims_id_seq", "last_value"),
            "referral_source": ("core_referrals.source", "archive_sha256"),
            "referral_bindings": ("core_referrals.bindings", "user_id"),
            "referral_contracts": ("core_referrals.contracts", "id"),
            "referral_history": ("core_referrals.history", "inviter"),
            "referral_events": ("core_referrals.events", "id"),
            "referral_moderation": ("core_referrals.moderation", "request_id"),
            "referral_event_journals": ("core_referrals.event_journals", "event_id"),
            "referral_events_sequence": ("core_referrals.events_id_seq", "last_value"),
        })
    fields = []
    for key, (relation, order) in relations.items():
        # PostgreSQL sequences have no composite row type for to_jsonb(t).
        # Keep the same physical fields that the old whole-row intent covered.
        row_json = ("jsonb_build_object('last_value',t.last_value,'log_cnt',t.log_cnt,'is_called',t.is_called)"
                    if key.endswith("sequence") else "to_jsonb(t)")
        # These are the only TIMESTAMPTZ columns in the pinned fresh49 seeds.
        # Keep every other column and the original UTC JSON representation.
        timestamp = {"ledger_accounts": "created_at", "trust_policy": "native_since",
                     "request_policy": "updated_at", "request_auto_policy": "updated_at"}.get(key)
        if timestamp:
            row_json += "||jsonb_build_object(" + sql_literal(timestamp) + "," + utc_timestamp_text_sql("t." + timestamp) + ")"
        fields.append(sql_literal(key) + ",COALESCE((SELECT jsonb_agg(" + row_json + " ORDER BY " +
                      order + ") FROM " + relation + " t),'[]'::jsonb)")
    return "SELECT encode(sha256(convert_to(jsonb_build_object(" + ",".join(fields) + ")::text,'UTF8')),'hex')"


def id_map_bytes(records: list[dict]) -> str:
    return "".join(canonical({"source_user_id": row["user_id"], "target_user_id": row["user_id"],
                              "target_personal_account_id": row["account_id"]}) + "\n" for row in records)


def validate_import_material(plan: dict, source_sha256: str, manifest: dict) -> None:
    records = plan.get("records")
    require(isinstance(records, list) and all(isinstance(row, dict) for row in records), "mapping_shape_invalid")
    require(hashlib.sha256(canonical({"source_sha256": source_sha256, "records": records}).encode()).hexdigest() == plan.get("mapping_sha256"), "mapping_hash_mismatch")
    summary = plan.get("summary")
    require(isinstance(summary, dict), "identity_summary_scope_invalid")
    for field in IDENTITY_ONLY_ZERO_FIELDS:
        require(type(summary.get(field)) is int and summary[field] == 0,
                "identity_summary_scope_invalid")
    for field in IDENTITY_ONLY_FORBIDDEN_NONZERO_FIELDS:
        if field in summary:
            require(type(summary[field]) is int and summary[field] == 0,
                    "identity_summary_scope_invalid")
    gaps = summary.get("functional_gaps")
    require(isinstance(gaps, list) and all(isinstance(gap, str) for gap in gaps)
            and IDENTITY_ONLY_REQUIRED_GAPS.issubset(gaps),
            "identity_summary_scope_invalid")
    ids = [positive_int(row.get("user_id"), "mapped_user_id_invalid") for row in records]
    require(ids == sorted(set(ids)) and [row.get("account_id") for row in records] == list(range(1, len(records) + 1)), "id_map_invalid")
    require(isinstance(manifest, dict) and manifest.get("format_version") == 2 and type(manifest.get("format_version")) is int
            and manifest.get("scope") == "identity-only" and manifest.get("source_archive_sha256") == source_sha256
            and manifest.get("identity_contract") == identity_contract() and manifest.get("developer_access_policy") == DEVELOPER_POLICY
            and manifest.get("initialized_core_seed_contract") == CORE_SEED_CONTRACT
            and manifest.get("import_tool_sha256") == sha256_file(Path(__file__).resolve())
            and manifest.get("import_contract_sha256") == hashlib.sha256(canonical(current_contract()).encode()).hexdigest()
            and manifest.get("mapping_sha256") == plan["mapping_sha256"] and manifest.get("summary") == plan.get("summary")
            and manifest.get("source_target_id_map_sha256") == hashlib.sha256(id_map_bytes(records).encode()).hexdigest(), "identity_manifest_mismatch")
    require(all(isinstance(manifest.get(field), str) and HEX.fullmatch(manifest[field]) for field in
                ("source_export_sha256", "source_baseline_proof_sha256")), "identity_manifest_proof_invalid")
    mapping_file_sha256 = hashlib.sha256((canonical(plan) + "\n").encode()).hexdigest()
    require(manifest.get("mapping_file_sha256", mapping_file_sha256) == mapping_file_sha256,
            "identity_manifest_mapping_file_mismatch")


def developer_decision(source: dict) -> tuple[bool, str]:
    """Mirror proven main facts; neither a balance nor generic paid status proves access.

    Main grants administrators access, then uses an explicit trust override
    before manual activation for ordinary users. Invalid or missing source
    policy fields fail closed. Paid activation needs its own authoritative
    net-credit/refund projection and is deliberately not inferred here.
    """
    role = source.get("role")
    if type(role) is not int or role not in (1, 10, 100):
        return False, "unknown_role_fail_closed"
    if role in (10, 100):
        return True, "admin_role_dormant"
    if "trust_level_override" not in source:
        return False, "override_source_missing"
    override = source["trust_level_override"]
    if override is not None:
        if type(override) is not int or override not in range(5):
            return False, "invalid_override_fail_closed"
        return (override > 0, "explicit_override_allow" if override else "explicit_override_deny")
    if "console_activated_at" not in source:
        return False, "manual_activation_source_missing"
    activated_at = source["console_activated_at"]
    if type(activated_at) is not int or not 0 <= activated_at <= 9223372036854775807:
        return False, "invalid_manual_activation_fail_closed"
    if activated_at > 0:
        return True, "approved_console_activation"
    return False, "paid_activation_unproven"


def positive_int(value: Any, code: str) -> int:
    require(type(value) is int and 0 < value <= 9223372036854775807, code)
    return value


def safe_argon2(password: Any) -> bool:
    """Only the current Rust default profile; never trust arbitrary PHC costs."""
    if not isinstance(password, str):
        return False
    match = ARGON2.fullmatch(password)
    if match is None:
        return False
    try:
        decoded = [base64.b64decode(part + "=" * (-len(part) % 4), validate=True) for part in match.groups()]
        return [len(part) for part in decoded] == [16, 32]
    except ValueError:
        return False


def text(value: Any, maximum: int, code: str, default: str | None = None) -> tuple[str, bool]:
    normalized = value is None and default is not None
    if normalized:
        value = default
    require(isinstance(value, str) and len(value) <= maximum and "\0" not in value, code)
    return value, normalized


def map_identity(users: list[dict], second_factors: list[dict], *, source_sha256: str,
                 password_login_enabled: bool = True) -> dict:
    """Pure deterministic mapper. Source dictionaries are never modified.

    Role/status uncertainty blocks an account instead of granting authority.
    All privileged accounts are blocked pending granular permission equivalence.
    Email verification cannot be inferred from a nonempty email, so email login
    is disabled. Service level remains zero. Developer access preserves only
    explicit override/manual facts, never an inferred paid entitlement. The
    original rows are preserved by the caller.
    """
    require(isinstance(source_sha256, str) and HEX.fullmatch(source_sha256) is not None, "source_hash_invalid")
    require(type(password_login_enabled) is bool, "password_login_policy_unknown")
    require(isinstance(users, list) and isinstance(second_factors, list), "source_shape_invalid")
    indexed = {}
    for user in users:
        require(isinstance(user, dict), "user_shape_invalid")
        user_id = positive_int(user.get("id"), "user_id_invalid")
        require(user_id not in indexed, "duplicate_user_id")
        indexed[user_id] = user
    factors: dict[int, bool] = {}
    unknown_factors: set[int] = set()
    for factor in second_factors:
        require(isinstance(factor, dict), "second_factor_shape_invalid")
        user_id = positive_int(factor.get("user_id"), "second_factor_user_id_invalid")
        require(user_id in indexed, "orphan_second_factor")
        require(type(factor.get("enabled")) is bool, "second_factor_policy_unknown")
        require(user_id not in factors, "duplicate_second_factor")
        factors[user_id] = factor["enabled"]
        if factor.get("policy_unknown"):
            unknown_factors.add(user_id)
    records, names, reasons, normalizations, developer_reasons = [], set(), Counter(), Counter(), Counter()
    for account_id, (user_id, source) in enumerate(sorted(indexed.items()), 1):
        username, _ = text(source.get("username"), 128, "username_invalid")
        require(bool(username.strip()), "username_invalid")
        require(username not in names, "duplicate_username")
        names.add(username)
        profile = {}
        normalized = []
        for field, limit, default in (("display_name", 256, ""), ("email", 320, ""), ("setting", 65536, "{}")):
            profile[field], changed = text(source.get(field), limit, field + "_invalid", default)
            if changed:
                normalized.append(field + "_null_default")
        registered_at = source.get("created_at")
        require(registered_at is None or (type(registered_at) is int and 0 <= registered_at <= 253402300799), "registration_time_invalid")
        auth_version = positive_int(source.get("auth_version"), "auth_version_invalid")
        blocked = []
        role = source.get("role")
        platform_role = {1: "user", 10: "admin", 100: "superadmin"}.get(role) if type(role) is int else None
        if platform_role is None:
            platform_role = "user"
            blocked.append("unknown_role")
        if role in (10, 100) and type(role) is int:
            blocked.append("privileged_permissions_unmapped")
        status = source.get("status")
        if type(status) is not int or status not in (1, 2):
            blocked.append("unknown_status")
        elif status == 2:
            blocked.append("source_disabled")
        source_deleted = source.get("deleted_at") not in (None, "")
        if source_deleted:
            blocked.append("source_deleted")
        # Archive source flags stay independent of the dormant live gate used for
        # privileged accounts pending offline owner activation.
        source_enabled = type(status) is int and status == 1 and not source_deleted
        active = not blocked
        second_factor = factors.get(user_id, False) or user_id in unknown_factors
        if second_factor:
            blocked.append("second_factor_unimplemented")
        if user_id in unknown_factors:
            blocked.append("second_factor_policy_unknown")
        password = source.get("password")
        require(password is None or (isinstance(password, str) and "\0" not in password), "password_source_invalid")
        legacy_bcrypt = isinstance(password, str) and BCRYPT.fullmatch(password) is not None
        known_argon2 = safe_argon2(password)
        verifier = password if legacy_bcrypt or known_argon2 else None
        if verifier is None:
            blocked.append("password_absent" if password in (None, "") else "password_format_unsupported")
        if not password_login_enabled:
            blocked.append("source_password_login_disabled")
        source_password_login_enabled = password_login_enabled
        # Source global policy, verifier availability and MFA are distinct facts.
        # Imported privileged accounts stay dormant even when source permits login.
        password_allowed = active and not second_factor and verifier is not None and password_login_enabled
        developer_granted, developer_reason = developer_decision(source)
        record = {
            "user_id": user_id, "account_id": account_id, "platform_role": platform_role,
            "registered_at": registered_at,
            "active": active, "auth_version": auth_version, "username": username,
            **profile, "email_login_allowed": False, "password_login_allowed": password_allowed,
            "second_factor_required": second_factor, "legacy_bcrypt_allowed": legacy_bcrypt,
            "password_hash": verifier, "blocked_reasons": sorted(blocked), "normalizations": normalized,
            "source_enabled": source_enabled, "source_deleted": source_deleted,
            "source_password_login_enabled": source_password_login_enabled,
            "developer_access_granted": developer_granted,
            "developer_access_source_reference": f"legacy:{source_sha256}:{developer_reason}",
            "developer_access_revision": 1, "developer_access_reason": developer_reason,
        }
        records.append(record)
        reasons.update(blocked)
        normalizations.update(normalized)
        developer_reasons.update([developer_reason])
    digest = hashlib.sha256(canonical({"source_sha256": source_sha256, "records": records}).encode()).hexdigest()
    return {"records": records, "mapping_sha256": digest, "summary": {
        "source_user_count": len(users), "mapped_user_count": len(records), "dropped_user_count": 0,
        "registration_time_known_count": sum(bool(row["registered_at"]) for row in records),
        "registration_time_unknown_count": sum(not row["registered_at"] for row in records),
        "active_user_count": sum(row["active"] for row in records),
        "password_login_allowed_count": sum(row["password_login_allowed"] for row in records),
        "password_verifier_count": sum(row["password_hash"] is not None for row in records),
        "blocked_user_count": sum(bool(row["blocked_reasons"]) for row in records),
        "blocked_reason_counts": dict(sorted(reasons.items())), "normalization_counts": dict(sorted(normalizations.items())),
        "email_login_allowed_count": 0, "sessions_imported": 0, "api_keys_imported": 0,
        "financial_rows_written": 0,
        "developer_access_granted_count": sum(row["developer_access_granted"] for row in records),
        "developer_access_denied_count": sum(not row["developer_access_granted"] for row in records),
        "developer_access_reason_counts": dict(sorted(developer_reasons.items())),
        "developer_access_policy": DEVELOPER_POLICY,
        "functional_gaps": ["legacy_email_verification_unproven", "legacy_service_levels_unmapped",
                            "legacy_paid_activation_unproven", "legacy_granular_admin_permissions_unmapped",
                            "legacy_oauth_and_passkeys_archived_only", "financial_data_not_migrated"],
    }}


def owned_path(path: Path, *, private: bool, directory: bool = False) -> Path:
    require(not path.is_symlink(), "path_symlink_rejected")
    info = path.stat()
    require(info.st_uid == os.getuid(), "path_owner_mismatch")
    require(stat.S_ISDIR(info.st_mode) if directory else stat.S_ISREG(info.st_mode), "path_type_invalid")
    require(info.st_mode & (0o077 if private else 0o022) == 0, "path_permissions_unsafe")
    return path


def private_backup_path(path: Path, *, directory: bool = False) -> Path:
    root = BACKUP_ROOT.resolve()
    require(path.is_absolute() and path.resolve().is_relative_to(root) and path.resolve() != root, "private_artifact_outside_backups")
    # Check every component below private evidence directory, not just the final path.
    for parent in reversed(path.parents):
        if parent != root and parent.is_relative_to(root):
            owned_path(parent, private=True, directory=True)
    return owned_path(path, private=True, directory=directory)


def write_private(directory: Path, name: str, value: str) -> Path:
    path = directory / name
    # Never replace an existing preview, even if a migration ID is reused.
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, "w", encoding="utf-8") as stream:
        stream.write(value)
        stream.flush()
        os.fsync(stream.fileno())
    return path


def load_receipt(path: Path, private: bool = False) -> dict:
    owned_path(path, private=private)
    with path.open() as stream:
        receipt = json.load(stream)
    require(isinstance(receipt, dict), "receipt_shape_invalid")
    return receipt


def socket_connection(connection: dict) -> dict:
    require(isinstance(connection, dict), "connection_receipt_invalid")
    socket_dir = Path(connection.get("socket_directory", ""))
    require(socket_dir.is_absolute(), "tcp_connection_rejected")
    owned_path(socket_dir, private=True, directory=True)
    port = connection.get("port")
    require(type(port) is int and 1 <= port <= 65535, "socket_port_missing_or_invalid")
    socket_file = socket_dir / (".s.PGSQL." + str(port))
    require(not socket_file.is_symlink(), "socket_symlink_rejected")
    socket_stat = socket_file.stat()
    require(stat.S_ISSOCK(socket_stat.st_mode) and socket_stat.st_uid == os.getuid(), "socket_owner_mismatch")
    pgpass = private_backup_path(Path(connection.get("pgpass_file", "")))
    require(IDENTIFIER.fullmatch(connection.get("database", "")) is not None, "database_name_invalid")
    _, login = connection_roles(connection)
    return {"PGHOST": str(socket_dir), "PGPORT": str(port), "PGDATABASE": connection["database"],
            "PGUSER": login, "PGPASSFILE": str(pgpass)}


def connection_roles(connection: dict) -> tuple[str, str]:
    owner = connection.get("role", "")
    require(isinstance(owner, str) and IDENTIFIER.fullmatch(owner) is not None, "database_role_invalid")
    require(("login_role" in connection) == ("set_role" in connection), "target_login_role_pair_required")
    login = connection.get("login_role", owner)
    require(isinstance(login, str) and IDENTIFIER.fullmatch(login) is not None, "database_login_role_invalid")
    if "set_role" in connection:
        require(connection["set_role"] == owner and login != owner, "target_set_role_invalid")
    return owner, login


def psql_options(connection: dict, *, readonly: bool) -> str:
    options = "-c default_transaction_read_only=" + ("on" if readonly else "off")
    if readonly:
        options += READONLY_PSQL_TIMEOUT_OPTIONS
    if "login_role" in connection or "set_role" in connection:
        owner, _ = connection_roles(connection)
        options += " -c role=" + owner
    return options


def psql(connection: dict, sql: str, *, readonly: bool, private_dir: Path, label: str) -> str:
    variables = socket_connection(connection)
    # Do not inherit PG* DSNs, service files, startup SQL, or ambient libpq options.
    environment = {key: value for key, value in os.environ.items() if not key.startswith("PG")}
    environment.update(variables)
    environment["PGCONNECT_TIMEOUT"] = "5"
    environment["PGAPPNAME"] = "lmm-offline-identity-import"
    environment["PGOPTIONS"] = psql_options(connection, readonly=readonly)
    try:
        result = subprocess.run(["psql", "-X", "--no-password", "-qAt", "-v", "ON_ERROR_STOP=1"],
                                input=sql, text=True, capture_output=True, env=environment, check=False,
                                timeout=PSQL_CLIENT_TIMEOUT_SECONDS)
    except subprocess.TimeoutExpired as error:
        # TimeoutExpired may carry bytes even with text=True. Preserve partial
        # diagnostics only in private files; never echo SQL or source values.
        def partial_text(value: str | bytes | None) -> str:
            return value.decode("utf-8", errors="replace") if isinstance(value, bytes) else value or ""

        write_private(private_dir, label + ".stdout.private.txt", partial_text(error.stdout))
        write_private(private_dir, label + ".stderr.private.txt", partial_text(error.stderr))
        suffix = "_database_command_timed_out" if readonly else "_database_command_timed_out_result_unconfirmed"
        raise MappingError(label + suffix) from None
    write_private(private_dir, label + ".stderr.private.txt", result.stderr)
    require(result.returncode == 0, label + "_database_command_failed")
    return result.stdout


IDENTITY_SQL = """SELECT json_build_object(
 'database',current_database(),'role',current_user,'session_user',session_user,
 'effective_role_can_login',(SELECT rolcanlogin FROM pg_roles WHERE rolname=current_user),
 'database_oid',(SELECT oid FROM pg_database WHERE datname=current_database()),
 'database_owner',(SELECT pg_get_userbyid(datdba) FROM pg_database WHERE datname=current_database()),
 'server_version_num',current_setting('server_version_num'),
 'postmaster_started_at',pg_postmaster_start_time()::text,
 'server_instance_nonce',current_setting('lmm.migration_instance_nonce',true),
 'port',current_setting('port')::int,
 'tcp_connection',inet_server_addr() IS NOT NULL,
 'read_only',current_setting('transaction_read_only'),
 'datcollate',(SELECT datcollate FROM pg_database WHERE datname=current_database()),
 'datctype',(SELECT datctype FROM pg_database WHERE datname=current_database()),
 'datcollversion',(SELECT datcollversion FROM pg_database WHERE datname=current_database()));"""


def verify_instance(identity: dict, connection: dict, expected: dict, *, source: bool) -> None:
    if source:
        require("login_role" not in connection and "set_role" not in connection, "source_role_switch_forbidden")
    require(isinstance(expected, dict), "instance_receipt_incomplete")
    expected = dict(expected)
    if "postmaster_started_at" not in expected:
        expected["postmaster_started_at"] = expected.get("postgres_start_time")
    require(identity.get("database") == connection["database"] and identity.get("role") == connection["role"], "database_identity_mismatch")
    # unix_socket_directories is a privileged setting. The owner attests the
    # offline listener in the receipt; libpq uses our checked owned Unix socket.
    listeners = ("",) if source else ("", "127.0.0.1")
    require(identity.get("tcp_connection") is False and expected.get("listen_addresses") in listeners, "database_not_offline")
    require(identity.get("port") == connection["port"], "database_socket_mismatch")
    require(isinstance(expected, dict) and all(key in expected for key in ("database_oid", "postmaster_started_at", "server_version_num")), "instance_receipt_incomplete")
    for key in ("database_oid", "server_version_num"):
        require(str(identity.get(key)) == str(expected[key]), "instance_receipt_mismatch")
    # PostgreSQL JSON timestamps use T/+HH:MM; ::text uses space/+HH.
    require(datetime.fromisoformat(identity["postmaster_started_at"]) == datetime.fromisoformat(expected["postmaster_started_at"]), "instance_receipt_mismatch")
    if source:
        require(identity.get("read_only") == "on", "source_not_readonly")
    else:
        require(expected.get("database_owner") == identity.get("database_owner") == connection["role"], "target_owner_mismatch")
        if "login_role" in connection or "set_role" in connection:
            owner, login = connection_roles(connection)
            require(identity.get("session_user") == login and identity.get("role") == owner
                    and identity.get("effective_role_can_login") is False, "target_session_role_mismatch")
        if expected.get("server_instance_nonce") is not None:
            # A custom GUC is an extra marker, not cryptographic attestation;
            # retain the independent socket/owner/OID/start/version checks.
            require(identity.get("server_instance_nonce") == expected["server_instance_nonce"], "target_instance_nonce_mismatch")


def export_source(receipt: dict, output: Path) -> tuple[dict, list[dict], list[dict], bool]:
    require(receipt.get("status") == "snapshot-restored-baseline-verified-frozen-and-classified", "source_receipt_status_invalid")
    archive = receipt.get("archive", {})
    archive_path = private_backup_path(Path(archive.get("path", "")))
    require(HEX.fullmatch(archive.get("sha256", "")) is not None and sha256_file(archive_path) == archive["sha256"], "source_archive_hash_mismatch")
    reader = receipt.get("readonly_reader", {})
    require("login_role" not in reader and "set_role" not in reader, "source_role_switch_forbidden")
    require(reader.get("verified_no_application_write_privileges") is True and reader.get("verified_no_security_definer_application_function_execute") is True and reader.get("row_security_off_guard_verified") is True, "source_reader_not_verified")
    schema = receipt.get("source", {}).get("schema", "")
    require(IDENTIFIER.fullmatch(schema) is not None, "source_schema_invalid")
    baselines = [Path(path) for path in receipt.get("private_metadata_files", [])
                 if Path(path).name == "native-table-row-count-baseline.jsonl"]
    require(len(baselines) == 1, "source_row_baseline_missing")
    baseline_path = private_backup_path(baselines[0])
    baseline_users = []
    for line in baseline_path.read_text().splitlines():
        if not line.startswith("{"):
            continue  # psql transaction status lines are not metadata rows.
        row = json.loads(line)
        if row.get("table") in ("users", schema + ".users"):
            baseline_users.append(row.get("row_count"))
    require(len(baseline_users) == 1 and type(baseline_users[0]) is int and baseline_users[0] >= 0, "source_row_baseline_invalid")
    # The snapshot owner records this identity after creating the readonly role.
    expected = dict(receipt.get("source_instance_identity", {}))
    identity = json.loads(psql(reader, "BEGIN READ ONLY; SET LOCAL timezone='UTC';\n" + IDENTITY_SQL + "\nCOMMIT;", readonly=True, private_dir=output, label="source-identity"))
    verify_instance(identity, reader, expected, source=True)
    # Fetch complete original user objects, not a lossy selected-field projection.
    # Secrets in these private objects must never reach stdout, fixtures or Git.
    sql = f"""BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;
SET LOCAL timezone='UTC';
SET LOCAL row_security=off;
SELECT json_build_object('kind','identity','row',({IDENTITY_SQL.removesuffix(';').removeprefix('SELECT ')}));
SELECT json_build_object('kind','schema_identity','row',json_build_object('schema',nspname,'schema_owner',pg_get_userbyid(nspowner))) FROM pg_namespace WHERE nspname='{schema}';
SELECT json_build_object('kind','user_count','row',count(*)) FROM "{schema}".users;
SELECT json_build_object('kind','user','row',row_to_json(u)) FROM "{schema}".users u ORDER BY id;
SELECT json_build_object('kind','second_factor','row',json_build_object('user_id',user_id,'is_enabled',is_enabled,'deleted_at',deleted_at)) FROM "{schema}".two_fas ORDER BY user_id;
SELECT json_build_object('kind','password_policy','row',json_build_object('key',key,'value',value)) FROM "{schema}".options WHERE key='PasswordLoginEnabled';
COMMIT;
"""
    raw = psql(reader, sql, readonly=True, private_dir=output, label="source-export")
    write_private(output, "source-identity-export.private.jsonl", raw)
    users, factors, policies, counts = [], [], [], []
    identities, schemas = 0, 0
    for line in raw.splitlines():
        tagged = json.loads(line)
        kind, row = tagged["kind"], tagged["row"]
        if kind == "identity":
            identities += 1
            verify_instance(row, reader, expected, source=True)
            require(row == identity, "source_instance_changed_during_export")
        elif kind == "schema_identity":
            schemas += 1
            require(row.get("schema") == schema and row.get("schema_owner") == expected.get("schema_owner"), "source_schema_owner_mismatch")
        elif kind == "user_count":
            counts.append(row)
        elif kind == "user":
            users.append(row)
        elif kind == "password_policy":
            policies.append(row)
        elif kind == "second_factor" and row["deleted_at"] is None:
            # An unknown MFA flag never silently becomes false.
            factors.append({"user_id": row["user_id"], "enabled": row["is_enabled"] is not False,
                            "policy_unknown": type(row["is_enabled"]) is not bool})
    require(identities == 1 and schemas == 1 and counts == [len(users)], "source_export_completeness_mismatch")
    require(counts == baseline_users, "source_snapshot_baseline_mismatch")
    require(len(policies) <= 1, "duplicate_global_password_policy")
    password_enabled = not policies or policies[0]["value"] == "true"
    write_private(output, "source-instance.private.json", canonical(identity) + "\n")
    write_private(output, "source-baseline-proof.private.json", canonical({
        "row_baseline_sha256": sha256_file(baseline_path), "source_user_row_count_matches_snapshot_baseline": True,
    }) + "\n")
    return identity, users, factors, password_enabled


def sql_literal(value: str) -> str:
    return "'" + value.replace("'", "''") + "'"


def build_import_sql(plan: dict, *, migration_id: str, source_sha256: str,
                     target_contract: dict, manifest: dict, target_identity: dict,
                     mapping_file_sha256: str | None = None) -> str:
    require(MIGRATION_ID.fullmatch(migration_id) is not None, "migration_id_invalid")
    require(HEX.fullmatch(source_sha256) is not None, "source_hash_invalid")
    if mapping_file_sha256 is None:
        # Exact bytes of identity-mapping.private.json (canonical plan + newline).
        mapping_file_sha256 = hashlib.sha256((canonical(plan) + "\n").encode()).hexdigest()
    require(isinstance(mapping_file_sha256, str) and HEX.fullmatch(mapping_file_sha256) is not None
            and mapping_file_sha256 == hashlib.sha256((canonical(plan) + "\n").encode()).hexdigest(),
            "mapping_file_hash_invalid")
    expected_contract = identity_contract()
    require(isinstance(target_contract, dict) and all(target_contract.get(key) == value for key, value in expected_contract.items()), "target_contract_invalid")
    require(type(target_contract.get("identity_version")) is int, "target_contract_invalid")
    contract = current_contract()
    for key in ("component_fingerprints", "catalog_query_sha256", "oauth_catalog_query_sha256",
                "installer_contract_sha256", "source_file_hashes"):
        require(target_contract.get(key) == contract[key], "target_assembled_contract_invalid")
    for key in ("import_profile", "source_schema_revision", "compiled_manifest_sha256", "fresh_catalog_sha256", "seed_query_sha256"):
        require(target_contract.get(key) == contract.get(key), "target_assembled_contract_invalid")
    catalog_hash = target_contract.get("catalog_fingerprint_sha256", "")
    require(isinstance(catalog_hash, str) and HEX.fullmatch(catalog_hash) is not None, "target_catalog_hash_invalid")
    if contract.get("import_profile") == CORE54_SEED_CONTRACT:
        require(catalog_hash == contract["fresh_catalog_sha256"], "target_fresh54_catalog_mismatch")
    validate_import_material(plan, source_sha256, manifest)
    require(isinstance(target_identity, dict), "target_identity_incomplete")
    target_identity = dict(target_identity)
    # PostgreSQL's JSON encoder represents oid as a decimal string.
    if isinstance(target_identity.get("database_oid"), str) and target_identity["database_oid"].isascii() and target_identity["database_oid"].isdigit():
        target_identity["database_oid"] = int(target_identity["database_oid"])
    for key in ("database", "role", "database_owner", "postmaster_started_at", "server_instance_nonce"):
        require(isinstance(target_identity.get(key), str) and bool(target_identity[key]) and "\0" not in target_identity[key], "target_identity_incomplete")
    require(IDENTIFIER.fullmatch(target_identity["database"]) is not None and IDENTIFIER.fullmatch(target_identity["role"]) is not None
            and target_identity["database_owner"] == target_identity["role"], "target_identity_invalid")
    login = target_identity.get("session_user")
    require(isinstance(login, str) and IDENTIFIER.fullmatch(login) is not None
            and login != target_identity["role"], "target_independent_installer_required")
    role_guard = f"""
 IF current_user<>{sql_literal(target_identity['role'])} OR session_user<>{sql_literal(login)}
 OR (SELECT pg_get_userbyid(datdba) FROM pg_database WHERE datname=current_database())<>{sql_literal(target_identity['role'])}
 OR EXISTS(SELECT 1 FROM pg_roles WHERE rolname={sql_literal(target_identity['role'])}
  AND (rolcanlogin OR rolsuper OR rolcreaterole OR rolcreatedb OR rolreplication OR rolbypassrls))
 OR NOT EXISTS(SELECT 1 FROM pg_roles WHERE rolname={sql_literal(login)} AND rolcanlogin
  AND NOT (rolsuper OR rolcreaterole OR rolcreatedb OR rolreplication OR rolbypassrls))
 OR EXISTS(SELECT 1 FROM pg_roles WHERE rolcanlogin AND NOT rolsuper AND rolname<>session_user
  AND has_database_privilege(oid,current_database(),'CONNECT'))
 OR NOT pg_has_role(session_user,current_user,'MEMBER') THEN
  RAISE EXCEPTION 'target session or effective owner mismatch';
 END IF;
"""
    positive_int(target_identity.get("database_oid"), "target_identity_invalid")
    require(type(target_identity.get("port")) is int and 0 < target_identity["port"] <= 65535
            and str(target_identity.get("server_version_num", "")).isdigit(), "target_identity_invalid")
    datetime.fromisoformat(target_identity["postmaster_started_at"])
    fingerprint = expected_contract["identity_fingerprint_sha256"]
    payload = sql_literal(canonical(plan["records"]))
    run_id, source_hash, mapping_hash, mapping_file_hash = map(
        sql_literal, (migration_id, source_sha256, plan["mapping_sha256"], mapping_file_sha256))
    # Placeholder digested with the pre-seal SQL body, then replaced. This is the
    # SQL-body digest contract: never claim it is the sealed complete-file SHA256.
    sql_body_digest_placeholder = "__LMM_A2_SQL_BODY_SHA256__"
    run_manifest = sql_literal(canonical(manifest))
    # A marker absent from all user material prevents replacing source strings
    # that happen to contain 64 zeroes while sealing the two digest positions.
    while sql_body_digest_placeholder in payload + run_manifest + str(target_identity) + migration_id:
        sql_body_digest_placeholder += "X"
    privileged_count = sum(1 for row in plan["records"] if row["platform_role"] in ("admin", "superadmin"))
    allow_core_seed = target_contract.get("allow_initialized_core_seed") is True
    # There is only one supported target here: the complete fresh Core49
    # installer. Deleting its seeds cannot opt into an older empty/ledger-only
    # contract, even if its catalog definition hash still matches.
    require(allow_core_seed and "allow_initialized_zero_ledger_seed" not in target_contract,
            "target_fresh49_seed_contract_required")
    if allow_core_seed:
        require(target_contract.get("initialized_core_seed_contract") == CORE_SEED_CONTRACT, "target_seed_contract_invalid")
        require(isinstance(target_contract.get("initialized_core_seed_sha256"), str)
                and HEX.fullmatch(target_contract["initialized_core_seed_sha256"]) is not None,
                "target_seed_hash_required")
    allow_seed = target_contract.get("allow_initialized_zero_ledger_seed") is True or allow_core_seed
    seed_exclusion = "AND NOT (n.nspname='core_billing' AND c.relname IN ('ledger_accounts','balance_state'))" if allow_seed else ""
    if allow_core_seed:
        seed_exclusion += " AND NOT (n.nspname='core_events' AND c.relname='capacity') AND NOT (n.nspname='public' AND c.relname='routing_active_release') AND NOT (n.nspname='core_abuse' AND c.relname='settings') AND NOT (n.nspname='core_oauth' AND c.relname='contract')"
        seed_exclusion += " AND NOT (n.nspname='core_humangrant' AND c.relname='human_grant_gate') AND NOT (n.nspname='core_gift' AND c.relname IN ('risk_key','policy')) AND NOT (n.nspname='core_tool_market' AND c.relname='credit_policy')"
        seed_exclusion += " AND NOT (n.nspname='core_trust_pricing' AND c.relname='policies')"
        seed_exclusion += " AND NOT (n.nspname='core_request_limits' AND c.relname IN ('policy','auto_policy')) AND NOT (n.nspname='core_promotions' AND c.relname='secrets') AND NOT (n.nspname='core_channels' AND c.relname='projection_state')"
    seed_guard = """
 IF (SELECT count(*) FROM core_billing.ledger_accounts)<>2
 OR (SELECT count(*) FROM core_billing.balance_state)<>2
 OR (SELECT count(DISTINCT bucket) FROM core_billing.ledger_accounts)<>2
 OR NOT EXISTS(SELECT 1 FROM core_billing.ledger_accounts WHERE id=1 AND bucket='clearing')
 OR NOT EXISTS(SELECT 1 FROM core_billing.ledger_accounts WHERE id=2 AND bucket='revenue')
 OR NOT EXISTS(SELECT 1 FROM core_billing.ledger_accounts_id_seq WHERE last_value=2 AND is_called)
 OR EXISTS(SELECT 1 FROM core_billing.ledger_accounts a LEFT JOIN core_billing.balance_state b ON b.ledger_account_id=a.id
  WHERE a.owner_account_id IS NOT NULL OR a.bucket NOT IN ('clearing','revenue') OR a.unit<>'credit_500k_usd'
  OR b.ledger_account_id IS NULL OR b.balance_units<>0 OR b.revision<>0) THEN
  RAISE EXCEPTION 'non-canonical initialized ledger seed';
 END IF;
 SELECT jsonb_build_object('accounts',(SELECT jsonb_agg(to_jsonb(a) ORDER BY id) FROM core_billing.ledger_accounts a),
  'balances',(SELECT jsonb_agg(to_jsonb(b) ORDER BY ledger_account_id) FROM core_billing.balance_state b),
  'ledger_sequence',(SELECT jsonb_build_object('last_value',s.last_value,'log_cnt',s.log_cnt,'is_called',s.is_called) FROM core_billing.ledger_accounts_id_seq s)) INTO seed_before;
""" if allow_seed else ""
    if allow_core_seed:
        seed_guard += """
 IF (SELECT count(*) FROM core_events.capacity)<>1
 OR NOT EXISTS(SELECT 1 FROM core_events.capacity WHERE singleton AND max_events=100000 AND max_bytes=268435456 AND pending_events=0 AND pending_bytes=0)
 OR (SELECT count(*) FROM public.routing_active_release)<>1
 OR NOT EXISTS(SELECT 1 FROM public.routing_active_release WHERE singleton AND config_version IS NULL)
 OR (SELECT count(*) FROM core_abuse.settings)<>1
 OR NOT EXISTS(SELECT 1 FROM core_abuse.settings WHERE singleton AND revision=0 AND document='{"version":1,"policies":[]}'::jsonb)
 OR (SELECT count(*) FROM core_humangrant.human_grant_gate)<>1
 OR NOT EXISTS(SELECT 1 FROM core_humangrant.human_grant_gate WHERE id=1 AND revision=1)
 OR (SELECT count(*) FROM core_gift.risk_key)<>1
 OR NOT EXISTS(SELECT 1 FROM core_gift.risk_key WHERE singleton AND octet_length(secret)=32)
 OR (SELECT count(*) FROM core_gift.policy)<>1
 OR NOT EXISTS(SELECT 1 FROM core_gift.policy WHERE singleton AND NOT enabled AND exclusive_cap=0)
 OR (SELECT count(*) FROM core_tool_market.credit_policy)<>1
 OR NOT EXISTS(SELECT 1 FROM core_tool_market.credit_policy WHERE singleton AND version=1
  AND platform_account_id IS NULL AND platform_user_id IS NULL AND NOT enabled
  AND updated_by IS NULL AND updated_at=0 AND fee_bps=0) THEN
  RAISE EXCEPTION 'non-canonical initialized core seed';
 END IF;
 seed_before := seed_before || jsonb_build_object('event_capacity',(SELECT to_jsonb(c) FROM core_events.capacity c),
  'routing_release',(SELECT to_jsonb(r) FROM public.routing_active_release r),
  'abuse_settings',(SELECT to_jsonb(s) FROM core_abuse.settings s),
  'oauth_contract',(SELECT to_jsonb(c) FROM core_oauth.contract c),
  'human_grant_gate',(SELECT to_jsonb(c) FROM core_humangrant.human_grant_gate c),
  'gift_risk_key',(SELECT to_jsonb(c) FROM core_gift.risk_key c),
  'gift_policy',(SELECT to_jsonb(c) FROM core_gift.policy c),
  'credit_policy',(SELECT to_jsonb(c) FROM core_tool_market.credit_policy c));
"""
        seed_guard += """
 IF (SELECT count(*) FROM core_request_limits.policy)<>1
 OR NOT EXISTS(SELECT 1 FROM core_request_limits.policy WHERE singleton AND revision=1 AND NOT enabled
  AND window_seconds=60 AND user_limit IS NULL AND group_limits='{}'::jsonb)
 OR (SELECT count(*) FROM core_request_limits.auto_policy)<>1
 OR NOT EXISTS(SELECT 1 FROM core_request_limits.auto_policy WHERE singleton AND revision=1 AND cardinality(groups)=0)
 OR (SELECT count(*) FROM core_promotions.secrets)<>1
 OR NOT EXISTS(SELECT 1 FROM core_promotions.secrets WHERE singleton AND octet_length(code_key)=32)
 OR (SELECT count(*) FROM core_channels.projection_state)<>1
 OR NOT EXISTS(SELECT 1 FROM core_channels.projection_state WHERE singleton AND ready) THEN
  RAISE EXCEPTION 'non-canonical initialized completion seed';
 END IF;
"""
        seed_guard += " IF " + oauth_seed_guard_sql(contract) + " THEN RAISE EXCEPTION 'non-canonical initialized OAuth seed'; END IF;\n"
        seed_guard += " IF " + trust_policy_seed_guard_sql(target_contract) + " THEN RAISE EXCEPTION 'non-canonical initialized trust seed'; END IF;\n"
        seed_guard += " IF (" + fresh_seed_hash_query() + ") IS DISTINCT FROM " + sql_literal(target_contract["initialized_core_seed_sha256"]) + " THEN RAISE EXCEPTION 'initialized seed receipt mismatch'; END IF;\n"
        seed_guard += " seed_before := seed_before || jsonb_build_object('trust_policy',(" + trust_policy_seed_query() + "));\n"
    seed_readback = """
 IF seed_before - ARRAY['event_capacity','routing_release','abuse_settings','oauth_contract','human_grant_gate','gift_risk_key','gift_policy','credit_policy','trust_policy'] IS DISTINCT FROM jsonb_build_object(
  'accounts',(SELECT jsonb_agg(to_jsonb(a) ORDER BY id) FROM core_billing.ledger_accounts a),
  'balances',(SELECT jsonb_agg(to_jsonb(b) ORDER BY ledger_account_id) FROM core_billing.balance_state b),
  'ledger_sequence',(SELECT jsonb_build_object('last_value',s.last_value,'log_cnt',s.log_cnt,'is_called',s.is_called) FROM core_billing.ledger_accounts_id_seq s)) THEN
  RAISE EXCEPTION 'initialized ledger seed changed';
 END IF;
""" if allow_seed else ""
    if allow_core_seed:
        seed_readback += """
 IF seed_before->'event_capacity' IS DISTINCT FROM (SELECT to_jsonb(c) FROM core_events.capacity c)
 OR seed_before->'routing_release' IS DISTINCT FROM (SELECT to_jsonb(r) FROM public.routing_active_release r)
 OR seed_before->'abuse_settings' IS DISTINCT FROM (SELECT to_jsonb(s) FROM core_abuse.settings s)
 OR seed_before->'oauth_contract' IS DISTINCT FROM (SELECT to_jsonb(c) FROM core_oauth.contract c)
 OR seed_before->'human_grant_gate' IS DISTINCT FROM (SELECT to_jsonb(c) FROM core_humangrant.human_grant_gate c)
 OR seed_before->'gift_risk_key' IS DISTINCT FROM (SELECT to_jsonb(c) FROM core_gift.risk_key c)
 OR seed_before->'gift_policy' IS DISTINCT FROM (SELECT to_jsonb(c) FROM core_gift.policy c)
 OR seed_before->'credit_policy' IS DISTINCT FROM (SELECT to_jsonb(c) FROM core_tool_market.credit_policy c) THEN
  RAISE EXCEPTION 'initialized core seed changed';
 END IF;
"""
        seed_readback += " IF seed_before->'trust_policy' IS DISTINCT FROM (" + trust_policy_seed_query() + ") THEN RAISE EXCEPTION 'initialized trust seed changed'; END IF;\n"
        seed_readback += " IF (" + fresh_seed_hash_query() + ") IS DISTINCT FROM " + sql_literal(target_contract["initialized_core_seed_sha256"]) + " THEN RAISE EXCEPTION 'initialized seed receipt changed'; END IF;\n"
    application_tables = "c.relkind IN ('r','p') AND n.nspname NOT IN ('pg_catalog','information_schema','core_meta','core_migration') AND left(n.nspname,3)<>'pg_'"
    # No persistent DDL and no finance writes. Lock all owned business tables so
    # emptiness, inserts, readback and receipt are one SERIALIZABLE transaction.
    sql = f"""BEGIN ISOLATION LEVEL SERIALIZABLE;
SET LOCAL timezone='UTC';
SET LOCAL synchronous_commit=on;
SET LOCAL standard_conforming_strings=on;
SET LOCAL row_security=off;
SET LOCAL search_path=pg_catalog,public;
SET LOCAL lock_timeout='5s';
SET LOCAL statement_timeout='60s';
SELECT pg_advisory_xact_lock(741803121);
DO $lmm_identity_import$
DECLARE payload JSONB := {payload}::jsonb;
 previous core_migration.import_runs%ROWTYPE;
 table_row RECORD;
 occupied BOOLEAN;
 expected_count BIGINT := jsonb_array_length(payload);
 seed_before JSONB;
 sequence_before JSONB := '{{}}'::jsonb;
 sequence_after JSONB;
 sequence_value JSONB;
 sequence_row RECORD;
 actual_catalog TEXT;
BEGIN
 {role_guard}
 IF current_database()<>{sql_literal(target_identity['database'])} OR current_user<>{sql_literal(target_identity['role'])}
 OR (SELECT oid::bigint FROM pg_database WHERE datname=current_database())<>{target_identity['database_oid']}
 OR (SELECT pg_get_userbyid(datdba) FROM pg_database WHERE datname=current_database())<>{sql_literal(target_identity['database_owner'])}
 OR pg_postmaster_start_time()<>{sql_literal(target_identity['postmaster_started_at'])}::timestamptz
 OR current_setting('server_version_num')<>{sql_literal(str(target_identity['server_version_num']))}
 OR current_setting('lmm.migration_instance_nonce',true) IS DISTINCT FROM {sql_literal(target_identity['server_instance_nonce'])}
 OR current_setting('port')::int<>{target_identity['port']} OR inet_server_addr() IS NOT NULL
 OR current_setting('listen_addresses') NOT IN ('','localhost','127.0.0.1') THEN
  RAISE EXCEPTION 'target instance identity mismatch';
 END IF;
 IF EXISTS(SELECT 1 FROM pg_stat_activity WHERE datid=(SELECT oid FROM pg_database WHERE datname=current_database()) AND pid<>pg_backend_pid()) THEN
  RAISE EXCEPTION 'target has other database clients';
 END IF;
 LOCK TABLE core_meta.schema_contract,core_meta.component_contract,core_meta.runtime_contract,{','.join(UPGRADE_RECEIPT_TABLES)},core_migration.import_runs IN ACCESS EXCLUSIVE MODE;
 IF (SELECT count(*) FROM core_meta.schema_contract)<>1 OR NOT EXISTS (SELECT 1 FROM core_meta.schema_contract WHERE singleton AND version={IDENTITY_VERSION} AND encode(fingerprint,'hex')={sql_literal(fingerprint)}) THEN
  RAISE EXCEPTION 'identity contract mismatch';
 END IF;
 IF {upgrade_receipts_guard_sql()} THEN
  RAISE EXCEPTION 'fresh target has upgrade receipts';
 END IF;
 IF EXISTS(SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
  WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_'
  AND c.relkind IN ('r','p','S','v','m','f') AND pg_get_userbyid(c.relowner)<>current_user) THEN
  RAISE EXCEPTION 'target object owner mismatch';
 END IF;
 IF EXISTS(SELECT 1 FROM pg_namespace n WHERE n.nspname NOT IN ('pg_catalog','information_schema','public')
  AND left(n.nspname,3)<>'pg_' AND pg_get_userbyid(n.nspowner)<>current_user)
 OR EXISTS(SELECT 1 FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace
  WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_'
  AND pg_get_userbyid(p.proowner)<>current_user)
 OR EXISTS(SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace,
  LATERAL aclexplode(c.relacl) a WHERE n.nspname NOT IN ('pg_catalog','information_schema')
  AND left(n.nspname,3)<>'pg_' AND a.grantee=0) THEN
  RAISE EXCEPTION 'target schema ownership or public privileges mismatch';
 END IF;
 FOR table_row IN SELECT n.nspname,c.relname FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
  WHERE {application_tables} ORDER BY n.nspname,c.relname
 LOOP EXECUTE format('LOCK TABLE %I.%I IN ACCESS EXCLUSIVE MODE',table_row.nspname,table_row.relname); END LOOP;
 SELECT ({catalog_hash_sql(contract)}) INTO actual_catalog;
 IF actual_catalog<>{sql_literal(catalog_hash)}
 OR (SELECT count(*) FROM core_meta.runtime_contract)<>1
 OR NOT EXISTS(SELECT 1 FROM core_meta.runtime_contract WHERE singleton AND encode(catalog_fingerprint,'hex')=actual_catalog)
 OR (SELECT jsonb_object_agg(name,encode(fingerprint,'hex')) FROM core_meta.component_contract) IS DISTINCT FROM {sql_literal(canonical(contract['component_fingerprints']))}::jsonb THEN
  RAISE EXCEPTION 'installed core contract or actual catalog mismatch';
 END IF;
 {seed_guard}
 FOR sequence_row IN SELECT n.nspname,c.relname,q.* FROM pg_sequence q JOIN pg_class c ON c.oid=q.seqrelid
  JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_'
  ORDER BY n.nspname,c.relname
 LOOP
  IF sequence_row.seqtypid<>'bigint'::regtype OR sequence_row.seqstart<>1 OR sequence_row.seqincrement<>1
   OR sequence_row.seqmin<>1 OR sequence_row.seqmax<>(CASE
    WHEN sequence_row.nspname='core_account_appeals' AND sequence_row.relname='requests_id_seq'
    THEN 9007199254740991 ELSE 9223372036854775807 END)
   OR sequence_row.seqcache<>1 OR sequence_row.seqcycle THEN
   RAISE EXCEPTION 'non-canonical sequence definition';
  END IF;
  EXECUTE format('SELECT jsonb_build_object(''last_value'',s.last_value,''log_cnt'',s.log_cnt,''is_called'',s.is_called) FROM %I.%I s',sequence_row.nspname,sequence_row.relname) INTO sequence_value;
  sequence_before := sequence_before || jsonb_build_object(sequence_row.nspname||'.'||sequence_row.relname,sequence_value);
  IF sequence_row.nspname||'.'||sequence_row.relname NOT IN ('core_identity.accounts_id_seq','core_identity.users_id_seq','core_billing.ledger_accounts_id_seq')
   AND ((sequence_value->>'last_value')::bigint<>1 OR (sequence_value->>'is_called')::boolean) THEN
   RAISE EXCEPTION 'non-canonical unused sequence state';
  END IF;
 END LOOP;
 SELECT * INTO previous FROM core_migration.import_runs WHERE migration_id={run_id} FOR UPDATE;
 IF FOUND THEN
  IF previous.source_archive_sha256<>{source_hash} OR previous.mapping_sha256<>{mapping_hash} OR previous.manifest<>{run_manifest}::jsonb THEN
   RAISE EXCEPTION 'migration receipt mismatch';
  END IF;
 ELSE
  IF EXISTS(SELECT 1 FROM core_migration.import_runs) THEN
   RAISE EXCEPTION 'target already belongs to another migration';
  END IF;
  IF (SELECT last_value<>1 OR is_called FROM core_identity.accounts_id_seq)
   OR (SELECT last_value<>1 OR is_called FROM core_identity.users_id_seq) THEN
   RAISE EXCEPTION 'non-canonical identity sequence state';
  END IF;
  FOR table_row IN SELECT n.nspname,c.relname FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
   WHERE {application_tables}
   {seed_exclusion}
  LOOP
   EXECUTE format('SELECT EXISTS(SELECT 1 FROM %I.%I)',table_row.nspname,table_row.relname) INTO occupied;
   IF occupied THEN RAISE EXCEPTION 'target business table is not empty'; END IF;
  END LOOP;
  -- Epoch zero explicitly marks an unknown legacy registration time; never use import time.
{ACCOUNT_IMPORT_SQL}
  INSERT INTO core_identity.users(id,personal_account_id,platform_role,active,auth_version)
   SELECT (r->>'user_id')::bigint,(r->>'account_id')::bigint,r->>'platform_role',(r->>'active')::boolean,(r->>'auth_version')::bigint FROM jsonb_array_elements(payload) r;
  INSERT INTO core_identity.user_profiles(user_id,username,display_name,email,email_login_allowed,setting,password_login_allowed,second_factor_required,legacy_bcrypt_allowed)
   SELECT (r->>'user_id')::bigint,r->>'username',r->>'display_name',r->>'email',false,r->>'setting',(r->>'password_login_allowed')::boolean,(r->>'second_factor_required')::boolean,(r->>'legacy_bcrypt_allowed')::boolean FROM jsonb_array_elements(payload) r;
  INSERT INTO core_identity.password_logins(login_name,user_id,password_hash)
   SELECT 'legacy-'||(r->>'user_id'),(r->>'user_id')::bigint,r->>'password_hash' FROM jsonb_array_elements(payload) r WHERE r->>'password_hash' IS NOT NULL;
  INSERT INTO core_identity.developer_access(user_id,granted,source_reference,revision)
   SELECT (r->>'user_id')::bigint,(r->>'developer_access_granted')::boolean,r->>'developer_access_source_reference',(r->>'developer_access_revision')::bigint FROM jsonb_array_elements(payload) r;
  INSERT INTO core_billing.account_policies(account_id,preference,revision)
   SELECT (r->>'account_id')::bigint,'wallet_only',1 FROM jsonb_array_elements(payload) r;
  -- Privileged facts bind live cluster identity and A2 inputs in this same
  -- transaction. a2_sql_sha256 starts as a zero placeholder in the pre-seal
  -- base SQL body; Python replaces it with SHA-256(pre-seal bytes) before seal.
  INSERT INTO core_identity.privileged_import_facts(
   user_id,archive_sha256,a2_semantic_sha256,a2_file_sha256,a2_sql_sha256,migration_id,
   system_id,database_oid,instance_nonce,import_run,source_enabled,source_deleted,
   password_verifier_recognized,password_login_enabled,password_hash_sha256,login_name_sha256,
   source_role,source_second_factor_required,source_auth_version)
  SELECT (r->>'user_id')::bigint,
   decode({source_hash},'hex'),decode({mapping_hash},'hex'),decode({mapping_file_hash},'hex'),
   decode('{sql_body_digest_placeholder}','hex'),{run_id},
   (SELECT substring(int8send(system_identifier) FROM 1 FOR 8)||decode(repeat('00',24),'hex') FROM pg_control_system()),
   (SELECT oid::bigint FROM pg_database WHERE datname=current_database()),
   decode(current_setting('lmm.migration_instance_nonce',true),'hex'),
   sha256(convert_to({run_id},'UTF8')),
   (r->>'source_enabled')::boolean,(r->>'source_deleted')::boolean,
   (r->>'password_hash') IS NOT NULL,(r->>'source_password_login_enabled')::boolean,
   CASE WHEN r->>'password_hash' IS NULL THEN decode(repeat('00',32),'hex') ELSE sha256(convert_to(r->>'password_hash','UTF8')) END,
   sha256(convert_to('legacy-'||(r->>'user_id'),'UTF8')),
   r->>'platform_role',(r->>'second_factor_required')::boolean,(r->>'auth_version')::bigint
  FROM jsonb_array_elements(payload) r
  WHERE r->>'platform_role' IN ('admin','superadmin');
  -- ALTER SEQUENCE RESTART is transactional; setval would survive a rollback.
  EXECUTE format('ALTER SEQUENCE %s RESTART WITH %s',pg_get_serial_sequence('core_identity.accounts','id'),COALESCE((SELECT max((r->>'account_id')::bigint)+1 FROM jsonb_array_elements(payload) r),1));
  EXECUTE format('ALTER SEQUENCE %s RESTART WITH %s',pg_get_serial_sequence('core_identity.users','id'),COALESCE((SELECT max((r->>'user_id')::bigint)+1 FROM jsonb_array_elements(payload) r),1));
  INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest)
   VALUES({run_id},{source_hash},{mapping_hash},{run_manifest}::jsonb);
 END IF;
 IF (SELECT count(*) FROM core_identity.users)<>expected_count OR (SELECT count(*) FROM core_identity.accounts)<>expected_count
 OR (SELECT count(*) FROM core_identity.user_profiles)<>expected_count OR (SELECT count(*) FROM core_billing.account_policies)<>expected_count
 OR (SELECT count(*) FROM core_identity.developer_access)<>expected_count
 OR (SELECT count(*) FROM core_identity.password_logins)<>(SELECT count(*) FROM jsonb_array_elements(payload) r WHERE r->>'password_hash' IS NOT NULL)
 OR (SELECT count(*) FROM core_identity.privileged_import_facts)<>{privileged_count} THEN
  RAISE EXCEPTION 'identity row count mismatch';
 END IF;
 IF EXISTS(SELECT 1 FROM jsonb_array_elements(payload) r
  LEFT JOIN core_identity.users u ON u.id=(r->>'user_id')::bigint
  LEFT JOIN core_identity.accounts a ON a.id=u.personal_account_id
  LEFT JOIN core_identity.user_profiles p ON p.user_id=u.id
  LEFT JOIN core_identity.password_logins l ON l.user_id=u.id
  LEFT JOIN core_identity.developer_access d ON d.user_id=u.id
  LEFT JOIN core_billing.account_policies b ON b.account_id=a.id
  WHERE ROW(u.personal_account_id,u.platform_role,u.active,u.auth_version,a.kind,a.service_level,a.active,a.created_at,
   p.username,p.display_name,p.email,p.email_login_allowed,p.setting,p.password_login_allowed,p.second_factor_required,p.legacy_bcrypt_allowed,
   l.login_name,l.password_hash,b.preference,b.revision,d.granted,d.source_reference,d.revision) IS DISTINCT FROM
  ROW((r->>'account_id')::bigint,r->>'platform_role',(r->>'active')::boolean,(r->>'auth_version')::bigint,'personal'::text,0::smallint,(r->>'active')::boolean,to_timestamp(coalesce((r->>'registered_at')::bigint,0)),
   r->>'username',r->>'display_name',r->>'email',false,r->>'setting',(r->>'password_login_allowed')::boolean,(r->>'second_factor_required')::boolean,(r->>'legacy_bcrypt_allowed')::boolean,
   CASE WHEN r->>'password_hash' IS NULL THEN NULL ELSE 'legacy-'||(r->>'user_id') END,r->>'password_hash','wallet_only'::text,1::bigint,
   (r->>'developer_access_granted')::boolean,r->>'developer_access_source_reference',(r->>'developer_access_revision')::bigint)) THEN
  RAISE EXCEPTION 'identity exact readback mismatch';
 END IF;
 FOR table_row IN SELECT n.nspname,c.relname FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
  WHERE {application_tables}
  AND NOT (n.nspname='core_identity' AND c.relname IN ('accounts','users','user_profiles','password_logins','developer_access','privileged_import_facts'))
  AND NOT (n.nspname='core_billing' AND c.relname='account_policies')
  {seed_exclusion}
 LOOP
  EXECUTE format('SELECT EXISTS(SELECT 1 FROM %I.%I)',table_row.nspname,table_row.relname) INTO occupied;
  IF occupied THEN RAISE EXCEPTION 'non-import target table is not empty'; END IF;
 END LOOP;
 IF EXISTS(SELECT 1 FROM jsonb_array_elements(payload) r
  WHERE r->>'platform_role' IN ('admin','superadmin')
  AND NOT EXISTS(
   SELECT 1 FROM core_identity.privileged_import_facts f
   WHERE f.user_id=(r->>'user_id')::bigint
    AND encode(f.archive_sha256,'hex')={source_hash}
    AND encode(f.a2_semantic_sha256,'hex')={mapping_hash}
    AND encode(f.a2_file_sha256,'hex')={mapping_file_hash}
    AND f.migration_id={run_id}
    AND f.source_enabled=(r->>'source_enabled')::boolean
    AND f.source_deleted=(r->>'source_deleted')::boolean
    AND f.password_login_enabled=(r->>'source_password_login_enabled')::boolean
    AND f.password_verifier_recognized=((r->>'password_hash') IS NOT NULL)
    AND f.password_hash_sha256=CASE WHEN r->>'password_hash' IS NULL THEN decode(repeat('00',32),'hex') ELSE sha256(convert_to(r->>'password_hash','UTF8')) END
    AND f.login_name_sha256=sha256(convert_to('legacy-'||(r->>'user_id'),'UTF8'))
    AND f.source_role=r->>'platform_role'
    AND f.source_second_factor_required=(r->>'second_factor_required')::boolean
    AND f.source_auth_version=(r->>'auth_version')::bigint
    AND encode(f.import_run,'hex')=encode(sha256(convert_to({run_id},'UTF8')),'hex')
    AND encode(f.a2_sql_sha256,'hex')='{sql_body_digest_placeholder}'
    AND f.database_oid=(SELECT oid::bigint FROM pg_database WHERE datname=current_database())
    AND f.instance_nonce=decode(current_setting('lmm.migration_instance_nonce',true),'hex')
    AND f.system_id=(SELECT substring(int8send(system_identifier) FROM 1 FOR 8)||decode(repeat('00',24),'hex') FROM pg_control_system())
  )) THEN
  RAISE EXCEPTION 'privileged import facts mismatch';
 END IF;
 {seed_readback}
 sequence_after := '{{}}'::jsonb;
 FOR sequence_row IN SELECT n.nspname,c.relname FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
  WHERE c.relkind='S' AND n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_'
 LOOP
  EXECUTE format('SELECT jsonb_build_object(''last_value'',s.last_value,''log_cnt'',s.log_cnt,''is_called'',s.is_called) FROM %I.%I s',sequence_row.nspname,sequence_row.relname) INTO sequence_value;
  sequence_after := sequence_after || jsonb_build_object(sequence_row.nspname||'.'||sequence_row.relname,sequence_value);
 END LOOP;
 IF sequence_before - ARRAY['core_identity.accounts_id_seq','core_identity.users_id_seq'] IS DISTINCT FROM
    sequence_after - ARRAY['core_identity.accounts_id_seq','core_identity.users_id_seq'] THEN
  RAISE EXCEPTION 'non-import sequence changed';
 END IF;
 IF (SELECT last_value<>COALESCE((SELECT max((r->>'account_id')::bigint)+1 FROM jsonb_array_elements(payload) r),1) OR is_called FROM core_identity.accounts_id_seq)
 OR (SELECT last_value<>COALESCE((SELECT max((r->>'user_id')::bigint)+1 FROM jsonb_array_elements(payload) r),1) OR is_called FROM core_identity.users_id_seq) THEN
  RAISE EXCEPTION 'identity sequence readback mismatch';
 END IF;
 IF {upgrade_receipts_guard_sql()} THEN RAISE EXCEPTION 'fresh target upgrade receipts changed'; END IF;
 IF ({catalog_hash_sql(contract)}) IS DISTINCT FROM actual_catalog THEN RAISE EXCEPTION 'target catalog changed'; END IF;
 PERFORM pg_stat_clear_snapshot();
 IF EXISTS(SELECT 1 FROM pg_stat_activity WHERE datid=(SELECT oid FROM pg_database WHERE datname=current_database()) AND pid<>pg_backend_pid()) THEN
  RAISE EXCEPTION 'target has other database clients';
 END IF;
 {role_guard}
END $lmm_identity_import$;
SELECT json_build_object('status','identity-import-verified','migration_id',{run_id},'mapped_user_count',{len(plan['records'])});
COMMIT;
"""
    # SQL-body digest covers the pre-seal base import SQL with the zero digest
    # placeholder still present. After hashing, replace the placeholder; then
    # seal dollar-quote delimiters. Do not label this digest as complete-file SHA.
    require(sql.count(sql_body_digest_placeholder) == 2, "sql_body_digest_placeholder_invalid")
    sql_body_digest = hashlib.sha256(sql.replace(sql_body_digest_placeholder, "0" * 64).encode()).hexdigest()
    sql = sql.replace(sql_body_digest_placeholder, sql_body_digest)
    # Dollar quoting is lexical: a source string can close a fixed DO delimiter
    # even inside an inner SQL string. Choose a delimiter absent from all input.
    suffix = hashlib.sha256(sql.encode()).hexdigest()[:24]
    delimiter = "$lmm_import_" + suffix + "$"
    while delimiter in sql:
        suffix += "x"
        delimiter = "$lmm_import_" + suffix + "$"
    sql = sql.replace("DO $lmm_identity_import$", "DO " + delimiter, 1)
    before, found, after = sql.rpartition("END $lmm_identity_import$;")
    require(bool(found), "sql_template_invalid")
    return before + "END " + delimiter + ";" + after


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--source-receipt", type=Path, required=True)
    parser.add_argument("--target-receipt", type=Path, help="Owner-provisioned fresh54 receipt: status=initialized-empty-identity-target; migration_id; source_archive_sha256; connection={socket_directory,port,database,role:NOLOGIN-owner,login_role:independent-installer,set_role:owner,pgpass_file}; instance_identity={database_oid,database_owner,postmaster_started_at,server_version_num,server_instance_nonce,listen_addresses:empty_or_loopback}; contract=current_contract() plus catalog_fingerprint_sha256, allow_initialized_core_seed:true, initialized_core_seed_contract:CORE_SEED_CONTRACT, initialized_core_seed_sha256:fresh_seed_hash_query() result, trust_policy_seed:trust_policy_seed_query() result; application_or_worker_started:false; registry_provisioned:true. Capture and review fresh values; never relabel an old receipt.")
    parser.add_argument("--output", type=Path, required=True, help="New private directory under ~/.local/state/lmm-api; must not already exist")
    parser.add_argument("--migration-id", required=True)
    parser.add_argument("--apply", action="store_true", help="Explicitly execute the private SQL plan against the matching empty initialized target")
    parser.add_argument("--confirm-source-sha256", help="Required on apply; exact immutable archive SHA256")
    args = parser.parse_args()
    output = None
    apply_attempted = False
    try:
        require(MIGRATION_ID.fullmatch(args.migration_id) is not None, "migration_id_invalid")
        expected_contract = identity_contract()
        current_contract()  # Reject any unsupported assembled source before I/O.
        require(args.output.is_absolute() and args.output.resolve().is_relative_to(BACKUP_ROOT.resolve()), "private_artifact_outside_backups")
        private_backup_path(args.output.parent, directory=True)
        args.output.mkdir(mode=0o700)
        output = private_backup_path(args.output, directory=True)
        source = load_receipt(args.source_receipt)
        source_identity, users, factors, password_enabled = export_source(source, output)
        source_hash = source["archive"]["sha256"]
        plan = map_identity(users, factors, source_sha256=source_hash, password_login_enabled=password_enabled)
        mapping_document = canonical(plan) + "\n"
        write_private(output, "identity-mapping.private.json", mapping_document)
        mapping_file_sha256 = hashlib.sha256(mapping_document.encode()).hexdigest()
        id_map_path = write_private(output, "source-target-id-map.private.jsonl", id_map_bytes(plan["records"]))
        manifest = {"format_version": 2, "scope": "identity-only", "source_archive_sha256": source_hash,
                    "identity_contract": expected_contract, "developer_access_policy": DEVELOPER_POLICY,
                    "initialized_core_seed_contract": CORE_SEED_CONTRACT,
                    "import_tool_sha256": sha256_file(Path(__file__).resolve()),
                    "import_contract_sha256": hashlib.sha256(canonical(current_contract()).encode()).hexdigest(),
                    "source_export_sha256": sha256_file(output / "source-identity-export.private.jsonl"),
                    "source_baseline_proof_sha256": sha256_file(output / "source-baseline-proof.private.json"),
                    "source_target_id_map_sha256": sha256_file(id_map_path),
                    "mapping_sha256": plan["mapping_sha256"],
                    "mapping_file_sha256": mapping_file_sha256, "summary": plan["summary"]}
        validate_import_material(plan, source_hash, manifest)
        # Persist the exact replay materials before any possible target commit.
        write_private(output, "manifest.private.json", canonical(manifest) + "\n")
        public = {"status": "dry-run-mapped", "migration_id": args.migration_id, "apply": False,
                  "source_archive_sha256": source_hash, "mapping_sha256": plan["mapping_sha256"],
                  "mapping_file_sha256": mapping_file_sha256,
                  "identity_contract": expected_contract, **plan["summary"]}
        if args.target_receipt:
            target = load_receipt(args.target_receipt, private=True)
            require(target.get("status") == "initialized-empty-identity-target" and target.get("registry_provisioned") is True and target.get("application_or_worker_started") is False, "target_receipt_status_invalid")
            require(target.get("migration_id") == args.migration_id and target.get("source_archive_sha256") == source_hash, "source_target_receipt_mismatch")
            require(isinstance(target.get("contract"), dict) and all(target["contract"].get(key) == value for key, value in expected_contract.items())
                    and type(target["contract"].get("identity_version")) is int, "target_contract_invalid")
            connection = target.get("connection", {})
            instance = target.get("instance_identity", {})
            require(isinstance(instance, dict) and isinstance(instance.get("server_instance_nonce"), str) and bool(instance["server_instance_nonce"]), "target_instance_nonce_receipt_missing")
            # Reject stale DDL/seed receipts before even probing the target.
            _, login_role = connection_roles(connection)
            sql_identity = {**instance, "database": connection.get("database"), "role": connection.get("role"),
                            "session_user": login_role, "port": connection.get("port")}
            sql = build_import_sql(plan, migration_id=args.migration_id, source_sha256=source_hash,
                                  mapping_file_sha256=mapping_file_sha256,
                                  target_contract=target.get("contract", {}), manifest=manifest,
                                  target_identity=sql_identity)
            target_identity = json.loads(psql(connection, "BEGIN READ ONLY; SET LOCAL timezone='UTC';\n" + IDENTITY_SQL + "\nCOMMIT;", readonly=True, private_dir=output, label="target-identity"))
            verify_instance(target_identity, connection, target.get("instance_identity", {}), source=False)
            require((target_identity["database_oid"], target_identity["postmaster_started_at"]) != (source_identity["database_oid"], source_identity["postmaster_started_at"]), "source_target_same_database")
            for key in ("datcollate", "datctype", "datcollversion"):
                require(target_identity.get(key) == source_identity.get(key), "source_target_collation_mismatch")
            write_private(output, "identity-import.private.sql", sql)
            public["status"] = "dry-run-sql-plan-ready"
            if args.apply:
                require(args.confirm_source_sha256 == source_hash, "apply_source_confirmation_missing")
                require(sha256_file(Path(source["archive"]["path"])) == source_hash, "source_archive_changed_before_import")
                apply_attempted = True
                result = psql(connection, sql, readonly=False, private_dir=output, label="target-import")
                write_private(output, "target-import.private.jsonl", result)
                public.update(status="identity-import-verified", apply=True)
        else:
            require(not args.apply, "apply_requires_target_receipt")
        require(sha256_file(Path(source["archive"]["path"])) == source_hash, "source_archive_changed_during_import")
        write_private(output, "report.private.json", canonical(public) + "\n")
        print(canonical(public))
        return 0
    except (MappingError, OSError, ValueError, KeyError, TypeError) as error:
        # Database stderr can contain PII; it is already private, never echoed.
        code = str(error) if isinstance(error, MappingError) else type(error).__name__
        if output is not None:
            try:
                write_private(output, "failure.private.json", canonical({"status": "apply-outcome-unconfirmed" if apply_attempted else "aborted", "error_code": code}) + "\n")
            except OSError:
                pass
        print(canonical({"status": "apply-outcome-unconfirmed" if apply_attempted else "aborted", "error_code": code}), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
