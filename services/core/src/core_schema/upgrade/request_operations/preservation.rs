//! Snapshot the fixed predecessor objects, without adding new namespace facts.
use super::*;

pub(super) async fn permissions(
    c: &mut PgConnection,
    rels: &[i64],
    types: &[i64],
) -> Result<Vec<String>, UpgradeError> {
    let facts = super::super::finance::permissions(c, rels, types).await?;
    let mut original = Vec::new();
    for fact in facts {
        let value: serde_json::Value =
            serde_json::from_str(&fact).map_err(|_| UpgradeError::Storage)?;
        // The shared snapshot already binds relation/type OIDs. Schemas and
        // functions need the same predecessor boundary when adding namespaces.
        if matches!(value[0].as_str(), Some("schema" | "function"))
            && matches!(
                value[1].as_str(),
                Some("core_operations" | "core_request_limits")
            )
        {
            continue;
        }
        original.push(fact);
    }
    Ok(original)
}

pub(super) async fn sequences(c: &mut PgConnection) -> Result<Vec<String>, UpgradeError> {
    let names: Vec<String> = sqlx::query_scalar("SELECT format('%I.%I',n.nspname,c.relname) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_' AND c.relkind='S' ORDER BY n.nspname COLLATE \"C\",c.relname COLLATE \"C\"")
        .fetch_all(c).await?;
    if names.len() != 22 {
        return Err(reject("REQUEST_OPERATIONS_PREDECESSOR_SEQUENCES_CHANGED"));
    }
    Ok(names)
}

pub(in crate::core_schema::upgrade) async fn data(
    c: &mut PgConnection,
    tables: &[String],
    sequences: &[String],
) -> Result<BTreeMap<String, Vec<u8>>, UpgradeError> {
    let mut result = BTreeMap::new();
    for table in tables {
        if matches!(
            table.as_str(),
            "core_meta.component_contract" | "core_meta.runtime_contract"
        ) {
            continue;
        }
        // Table identifiers came from the pinned catalog, formatted by %I.
        sqlx::query(&format!("DECLARE request_operations_rows NO SCROLL CURSOR FOR SELECT json_build_array(encode(h,'hex'),count(*)::text)::text FROM (SELECT sha256(convert_to(to_jsonb(t)::text,'UTF8')) h FROM {table} t) q GROUP BY h ORDER BY h"))
            .execute(&mut *c).await?;
        let mut hash = Sha256::new();
        loop {
            let rows: Vec<String> =
                sqlx::query_scalar("FETCH FORWARD 256 FROM request_operations_rows")
                    .fetch_all(&mut *c)
                    .await?;
            if rows.is_empty() {
                break;
            }
            for row in rows {
                hash.update((row.len() as u64).to_be_bytes());
                hash.update(row.as_bytes());
            }
        }
        sqlx::query("CLOSE request_operations_rows")
            .execute(&mut *c)
            .await?;
        result.insert(table.clone(), hash.finalize().to_vec());
    }
    // Only the original 22 sequences are compared. New component sequences are
    // independently covered by the compiled DDL and successor catalog receipt.
    for sequence in sequences {
        let state: String = sqlx::query_scalar(&format!("SELECT json_build_array(last_value::text,log_cnt::text,is_called)::text FROM {sequence}"))
            .fetch_one(&mut *c).await?;
        result.insert(
            format!("sequence:{sequence}"),
            Sha256::digest(state.as_bytes()).to_vec(),
        );
    }
    Ok(result)
}
