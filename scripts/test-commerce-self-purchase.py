#!/usr/bin/env python3
"""Verify real PostgreSQL self-purchase ledger and the Core56 -> Core58 SQL delta.

Requires DATABASE_URL for a new, isolated PostgreSQL 18 database ending _test,
and psql on PATH. Uses only synthetic money, accounts and private fixture roles.
This runs the actual schema SQL; it does not compile Rust, invoke its offline
upgrade procedure, or establish Go-to-Core RPC/process acceptance.
"""
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import subprocess
from concurrent.futures import ThreadPoolExecutor


ROOT = Path(__file__).resolve().parents[1]
CORE = ROOT / "services/core"
PRE_MANIFEST = "f2340fb850e81f168ababe0301d80bd85769b235a8c5b5abb91f0bcd7429624e"
PRE_CATALOG = "46cbd8e0a9e9bb6c55cb899558b40fa8c8df1affa2e366cbbde733a8527bda43"


def need(condition, message):
    if not condition:
        raise AssertionError(message)


def sha(value):
    return hashlib.sha256(value).hexdigest()


def framed(rows):
    digest = hashlib.sha256()
    for row in rows:
        raw = row.encode()
        digest.update(len(raw).to_bytes(8, "big"))
        digest.update(raw)
    return digest.hexdigest()


def source_contract():
    source = (CORE / "src/core_schema.rs").read_text()
    declarations = source.split("const COMPONENTS:", 1)[1].split("\n];", 1)[0]
    names = re.findall(r'\(\s*"([a-z0-9-]+)",', declarations)
    combined = {
        "site-finance": ["site_statistics.sql"],
        "trust-pricing": ["trust_pricing.sql", "image_trust_extension.sql"],
        "channel-projection": ["channel_projection.sql", "channel_openings.sql"],
        "trust-policy-edit": ["trust_policy_edit.sql", "trust_policy_edit_proof.sql",
                              "trust_policy_edit_editor.sql", "trust_policy_upgrade.sql"],
        "channel-observations": ["channel_observations.sql", "channel_observations_upgrade.sql"],
        "payment-credit-quotes": ["payment_credit_quotes.sql", "payment_credit_quote_referrals.sql"],
    }
    language_component = 'subscription-pancake-language-optional'
    if 'checkins' in names:
        registry = re.findall(r'\(\s*"checkins"\s*,\s*([^)]*)\)', declarations)
        need(len(registry) == 1 and re.sub(r'\s+', '', registry[0]).rstrip(',') ==
             'crate::checkins::SCHEMA', 'check-in registry expression changed')
        producer = (CORE / 'src/checkins.rs').read_text()
        constants = re.findall(r'pub const SCHEMA:\s*&str\s*=\s*(.*?);', producer, re.S)
        need(len(constants) == 1 and re.sub(r'\s+', '', constants[0]) ==
             'include_str!("../schema/checkins.sql")', 'check-in schema constant changed')
    if language_component in names:
        registry = re.findall(
            r'\(\s*"subscription-pancake-language-optional"\s*,\s*include_str!\(\s*"([^"]+)"\s*\)\s*,?\s*\)',
            declarations)
        need(registry == ['../schema/subscription_pancake_language_optional.sql'],
             'pancake language registry expression changed')
    if "payment-credit-quotes" in names:
        registry = re.findall(r'\(\s*"payment-credit-quotes"\s*,\s*([^)]*)\)', declarations)
        need(len(registry) == 1 and re.sub(r'\s+', '', registry[0]).rstrip(',') ==
             'crate::payments::CREDIT_QUOTE_SCHEMA', 'credit quote registry expression changed')
        producer = (CORE / 'src/payments.rs').read_text()
        constants = re.findall(r'pub const CREDIT_QUOTE_SCHEMA:\s*&str\s*=\s*(.*?);', producer, re.S)
        expected = 'concat!(include_str!("../schema/payment_credit_quotes.sql"),"\\n",include_str!("../schema/payment_credit_quote_referrals.sql"))'
        need(len(constants) == 1 and re.sub(r'\s+', '', constants[0]) == expected,
             'credit quote composite constant changed')
    components = [(name, "\n".join(
        (CORE / "schema" / path).read_text()
        for path in combined.get(name, [name.replace("-", "_") + ".sql"])
    )) for name in names]
    alias = 'navigation_options::SUBSCRIPTION_STRIPE_WITH_UPGRADE_RECEIPT'
    if alias in declarations:
        producer = (CORE / 'src/core_schema/navigation_options.rs').read_text()
        constants = re.findall(r'const SUBSCRIPTION_STRIPE_WITH_UPGRADE_RECEIPT:\s*&str\s*=\s*(.*?);', producer, re.S)
        expected = 'concat!(include_str!("../../schema/subscription_stripe.sql"),"\\n",include_str!("upgrade/core59_to62_receipt.sql"))'
        need(len(constants) == 1 and re.sub(r'\s+', '', constants[0]) == expected,
             'Core62 composite constant changed: review resolver before capture')
        index = names.index('subscription-stripe')
        components[index] = ('subscription-stripe',
            (CORE/'schema/subscription_stripe.sql').read_bytes().decode('utf-8') + "\n" +
            (CORE/'src/core_schema/upgrade/core59_to62_receipt.sql').read_bytes().decode('utf-8'))
    metadata = [json.loads(re.search(r'const CONTRACT: &str = (".*");', source).group(1))]
    metadata += [(CORE / path).read_text() for path in [
        "schema/core_upgrade.sql", "schema/component_upgrade.sql",
        "schema/ledger_revision.sql", "src/core_schema/upgrade/console_receipt.sql",
        "schema/offline_upgrade.sql",
    ]]
    catalog = re.search(r'const CATALOG: &str = r#"([\s\S]*?)"#;', source).group(1)
    manifest = {
        "format_version": 1,
        "scope": "compiled-core-schema-contract",
        "components": [{"name": name, "raw_schema_sha256": sha(ddl.encode())}
                       for name, ddl in components],
        "identity": {"version": 6, "fingerprint_sha256": sha((CORE / "schema/identity.sql").read_bytes())},
        "installer_contract_sql_sha256": sha("".join(metadata).encode()),
        "catalog": {
            "query_utf8": catalog, "query_sha256": sha(catalog.encode()),
            "query_hash_domain": "sha256-raw-utf8-v1",
            "fingerprint_hash_domain": "sha256-u64be-byte-length-prefixed-utf8-v1",
            "definition_order": "postgresql-c-collation",
        },
    }
    return components, metadata, manifest


def manifest_hash(manifest):
    raw = json.dumps(manifest, separators=(",", ":"), ensure_ascii=False) + "\n"
    return sha(raw.encode())


def sql(body, role=None):
    prefix = f'SET ROLE "{role}";\n' if role else ""
    result = subprocess.run(
        ["psql", "-X", "-q", "-At", "-v", "ON_ERROR_STOP=1", os.environ["DATABASE_URL"]],
        input=prefix + body, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        timeout=30,
    )
    need(result.returncode == 0, "isolated PostgreSQL check failed:\n" + result.stderr)
    return result.stdout.strip()


def data_snapshot(tables, sequences, owner):
    statements = [f"SELECT jsonb_build_array('{table}',coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb))::text FROM {table} t"
                  for table in tables if table not in ("core_meta.component_contract", "core_meta.runtime_contract")]
    statements += [f"SELECT jsonb_build_array('{sequence}',last_value::text,log_cnt::text,is_called)::text FROM {sequence}"
                   for sequence in sequences]
    return framed(sorted(sql(";\n".join(statements), owner).splitlines()))


def permissions_snapshot(relations, types, functions, owner):
    # The same predecessor boundary as the Rust suffix engine: old objects,
    # including their exact owners, ACLs, columns, defaults and role membership.
    source = (CORE / "src/core_schema/upgrade/finance.rs").read_text()
    query = source.split("pub(super) async fn permissions(", 1)[1].split('r#"', 1)[1].split('"#', 1)[0]
    query = query.replace("$1", "ARRAY[" + ",".join(relations) + "]::bigint[]")
    query = query.replace("$2", "ARRAY[" + ",".join(types) + "]::bigint[]")
    query = query.replace("FROM pg_proc p JOIN ns n ON n.oid=p.pronamespace",
                          "FROM pg_proc p JOIN ns n ON n.oid=p.pronamespace WHERE p.oid IN (" + ",".join(functions) + ")")
    return framed(sql(query, owner).splitlines())


def post(action, key, owner):
    request = json.dumps({"scope": "core-commerce", "operation_key": key,
                          "reason": "Isolated concurrent self-purchase", "action": action}, separators=(",", ":"))
    return json.loads(sql("SELECT core_billing.post_ledger($request$" + request + "$request$::jsonb)", owner))


def concurrent(action, key, owner):
    with ThreadPoolExecutor(max_workers=8) as pool:
        receipts = list(pool.map(lambda _: post(action, key, owner), range(8)))
    need(all(row == receipts[0] for row in receipts), "concurrent replay returned different receipts")
    need(receipts[0]["status"] == "posted", "concurrent ledger operation did not post")
    return receipts[0]


def main():
    components, metadata, manifest = source_contract()
    need(len(components) == 58, "fixture requires the exact 58-component successor")
    predecessor = dict(manifest, components=manifest["components"][:56])
    need(manifest_hash(predecessor) == PRE_MANIFEST, "frozen 56-component source changed")
    target = json.loads((CORE / "src/core_schema/upgrade/commerce_display_rate_target.json").read_text())
    fresh = json.loads((CORE / "src/core_schema/upgrade/fresh_target.json").read_text())
    need(manifest_hash(manifest) == target["manifest_sha256"] and target == fresh,
         "source assembly differs from current frozen manifests")
    facts = json.loads(sql("SELECT jsonb_build_array(current_database(),current_setting('server_version_num')::int,"
                          "EXISTS(SELECT FROM pg_namespace n WHERE n.nspname NOT IN ('pg_catalog','information_schema','public') AND left(n.nspname,3)<>'pg_'),"
                          "EXISTS(SELECT FROM pg_class WHERE relnamespace='public'::regnamespace) OR EXISTS(SELECT FROM pg_proc WHERE pronamespace='public'::regnamespace))"))
    need(facts[0].endswith("_test") and 180000 <= facts[1] < 190000 and not facts[2] and not facts[3],
         "a fresh, isolated PostgreSQL 18 *_test database is required")
    suffix = secrets.token_hex(5)
    owner, runtime = "lmm_commerce_owner_" + suffix, "lmm_commerce_runtime_" + suffix
    database = '"' + facts[0].replace('"', '""') + '"'
    sql(f'CREATE ROLE "{owner}" NOLOGIN; CREATE ROLE "{runtime}" NOLOGIN; '
        f'ALTER DATABASE {database} OWNER TO "{owner}"; '
        f'GRANT USAGE,CREATE ON SCHEMA public TO "{owner}"')
    sql("BEGIN;\n" + "\n".join(ddl for _, ddl in components[:56]) + "\n" + "\n".join(metadata) + "\nCOMMIT;", owner)
    actual = framed(sql(manifest["catalog"]["query_utf8"], owner).splitlines())
    need(actual == PRE_CATALOG, "actual PostgreSQL predecessor differs from its frozen catalog")
    print("PASS: exact Core56 source manifest and PostgreSQL catalog", flush=True)
    sql("BEGIN;\n" + "\n".join(
        f"INSERT INTO core_meta.component_contract VALUES('{name}',decode('{sha(ddl.encode())}','hex'));"
        for name, ddl in components[:56]
    ) + f"\nINSERT INTO core_meta.runtime_contract(catalog_fingerprint) VALUES(decode('{PRE_CATALOG}','hex'));\n"
      "INSERT INTO core_console.options(key,value,revision) VALUES('USDExchangeRate','7.25',3);\n"
      "DO $$ DECLARE a bigint; BEGIN INSERT INTO core_identity.accounts(kind) VALUES('personal') RETURNING id INTO a; "
      "PERFORM core_billing.open_ledger_wallet(a); PERFORM core_billing.post_ledger(jsonb_build_object('scope','self-commerce-upgrade-test','operation_key','original-credit',"
      "'reason','Synthetic existing money to preserve across the SQL delta','action',jsonb_build_object('kind','credit','account_id',a,'amount_units',1000000,'payment_reference','synthetic-upgrade-credit'))); END $$;\nCOMMIT;", owner)
    sql(f'GRANT USAGE ON SCHEMA core_billing,core_console TO "{runtime}"; '
        f'GRANT SELECT ON core_billing.ledger_journals TO "{runtime}"; '
        f'GRANT EXECUTE ON FUNCTION core_billing.post_ledger(jsonb),core_billing.open_ledger_wallet(bigint) TO "{runtime}"', owner)
    application = "n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_'"
    tables = sql(f"SELECT format('%I.%I',n.nspname,c.relname) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE {application} AND c.relkind IN ('r','p') ORDER BY 1", owner).splitlines()
    sequences = sql(f"SELECT format('%I.%I',n.nspname,c.relname) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE {application} AND c.relkind='S' ORDER BY 1", owner).splitlines()
    relations = sql(f"SELECT c.oid::text FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE {application} ORDER BY c.oid", owner).splitlines()
    types = sql(f"SELECT t.oid::text FROM pg_type t JOIN pg_namespace n ON n.oid=t.typnamespace WHERE {application} ORDER BY t.oid", owner).splitlines()
    functions = sql(f"SELECT p.oid::text FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE {application} ORDER BY p.oid", owner).splitlines()
    before_data = data_snapshot(tables, sequences, owner)
    before_acl = permissions_snapshot(relations, types, functions, owner)
    sql("BEGIN;\n" + "\n".join(ddl for _, ddl in components[56:]) +
        f'\nGRANT EXECUTE ON FUNCTION core_console.write_display_exchange_rate(text) TO "{runtime}";\nCOMMIT;', owner)
    actual = framed(sql(manifest["catalog"]["query_utf8"], owner).splitlines())
    need(actual == target["catalog_sha256"], "upgraded catalog differs from frozen Core58 target")
    need(data_snapshot(tables, sequences, owner) == before_data, "SQL upgrade changed prior rows or sequence states")
    need(permissions_snapshot(relations, types, functions, owner) == before_acl, "SQL upgrade changed prior ownership or permissions")
    # Exercise the exact new function grant as the restricted runtime identity.
    result = json.loads(sql("SELECT jsonb_build_array(core_console.write_display_exchange_rate('7.5'),"
                            "has_table_privilege(current_user,'core_console.options','UPDATE'),"
                            "has_schema_privilege(current_user,'core_console','CREATE'))", runtime))
    need(result == [4, False, False], "new writer acquired direct options or DDL authority")
    print("PASS: Core56 -> Core58 SQL preserves all original rows, sequences and ACLs; new runtime grant is exact", flush=True)
    print(sql((CORE / "tests/core_schema/commerce_self_purchase.sql").read_text(), owner), flush=True)
    account = int(sql("SELECT min(id) FROM core_identity.accounts", owner))
    paid = concurrent({"kind": "reserve", "account_id": account, "amount_units": 500000}, "concurrent-pay", owner)
    settled = concurrent({"kind": "commerce_capture", "reservation_journal_id": paid["journal_id"],
                          "destination_account_id": account, "amount_units": 500000}, "concurrent-settle", owner)
    balances = "SELECT jsonb_agg(to_jsonb(b) ORDER BY ledger_account_id) FROM core_billing.balance_state b"
    before = sql(balances, owner)
    refunded = concurrent({"kind": "refund_commerce", "journal_id": settled["journal_id"]}, "concurrent-refund", owner)
    need(refunded["entries"] == [] and sql(balances, owner) == before,
         "concurrent self-refund changed a wallet, debt or balance revision")
    need(sql("SELECT count(*) FROM core_billing.ledger_journals WHERE operation_key IN ('concurrent-pay','concurrent-settle','concurrent-refund')", owner) == "3",
         "concurrent requests created duplicate journals")
    print("PASS: 8 concurrent pay/capture/refund replays each produced one original journal", flush=True)
    print(json.dumps({"evidence": "PostgreSQL SQL only; Rust offline API and RPC not executed",
                      "source_manifest_sha256": target["manifest_sha256"], "catalog_sha256": actual,
                      "preserved_data_sha256": before_data, "preserved_permissions_sha256": before_acl}), flush=True)


if __name__ == "__main__":
    main()
