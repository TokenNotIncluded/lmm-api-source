//! Offline channel proposals use the typed, reference-only model sidecar.
use super::{ModelConfig, ModelRouting, invalid_config, parse_document, validate_provider_routes};
use crate::routing::{CONTRACT_VERSION, ModelRoute, store::ReleaseDocument};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, Read, Write},
};
use toml_edit::{DocumentMut, Item, TableLike};

const MAX_SIDECAR: usize = 16 * 1024 * 1024;
const MAX_REQUEST: u64 = (MAX_SIDECAR as u64) * 3;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    model_config: String,
    plan: Value,
    #[serde(default)]
    local_http: bool,
}

fn invalid() -> io::Error {
    invalid_config("invalid channel proposal or model sidecar")
}

fn sidecar(text: &str) -> io::Result<ModelConfig> {
    if text.len() > MAX_SIDECAR {
        return Err(invalid());
    }
    parse_document(text.as_bytes())
}

fn release(model: ModelConfig, local_http: bool) -> io::Result<ReleaseDocument> {
    if model.version != 1 || local_http != model.external_models.is_empty() {
        return Err(invalid_config("model sidecar version or provider mode mismatch"));
    }
    let policy = crate::model_http::ProviderPolicy::new(model.external_models, model.input_token_ceiling)
        .map_err(|_| invalid_config("invalid external model context contracts"))?;
    let routing = model.routing.resolve(local_http)?;
    validate_provider_routes(&routing, &policy, model.input_token_ceiling)?;
    let enabled: BTreeSet<_> = routing.upstreams.iter().filter(|u| u.enabled)
        .map(|u| (u.credential.id, u.credential.revision)).collect();
    let supplied: BTreeSet<_> = model.credentials.iter()
        .map(|c| (c.reference.id, c.reference.revision)).collect();
    if enabled.is_empty() || enabled != supplied || supplied.len() != model.credentials.len() {
        return Err(invalid_config("model credential references do not match enabled upstreams"));
    }
    let doc = ReleaseDocument { contract_version: CONTRACT_VERSION, routing, pricing: model.prices };
    doc.clone().into_router().map_err(|_| invalid_config("invalid model routes or prices"))?;
    Ok(doc)
}

fn id(row: &dyn TableLike) -> Option<u64> {
    u64::try_from(row.get("id")?.as_integer()?).ok()
}

fn replace_fields(row: &mut dyn TableLike, old: &ModelRoute, new: &ModelRoute) -> io::Result<()> {
    if old == new {
        return Ok(());
    }
    let previous = serde_json::to_value(old).map_err(|_| invalid())?;
    let next = serde_json::to_value(new).map_err(|_| invalid())?;
    let fields = toml_edit::ser::to_document(new).map_err(|_| invalid())?.into_table().into_inline_table();
    for (key, value) in fields.iter() {
        if previous.get(key) != next.get(key) {
            row.insert(key, Item::Value(value.clone()));
        }
    }
    Ok(())
}

fn write_routes(text: &str, old: &[ModelRoute], new: &[ModelRoute], version: u64) -> io::Result<String> {
    let mut toml = text.parse::<DocumentMut>().map_err(|_| invalid())?;
    let routing = toml.get_mut("routing").and_then(Item::as_table_like_mut).ok_or_else(invalid)?;
    routing.insert("version", toml_edit::value(i64::try_from(version).map_err(|_| invalid())?));
    let rows = routing.get_mut("routes").ok_or_else(invalid)?;
    let before: BTreeMap<_, _> = old.iter().map(|row| (row.id, row)).collect();
    let after: BTreeMap<_, _> = new.iter().map(|row| (row.id, row)).collect();
    let update = |row: &mut dyn TableLike| -> io::Result<()> {
        let key = id(row).ok_or_else(invalid)?;
        replace_fields(row, before.get(&key).ok_or_else(invalid)?, after.get(&key).ok_or_else(invalid)?)
    };
    let added = new.iter().filter(|row| !before.contains_key(&row.id));
    if let Some(tables) = rows.as_array_of_tables_mut() {
        tables.retain(|row| id(row).is_some_and(|key| after.contains_key(&key)));
        for row in tables.iter_mut() {
            update(row)?;
        }
        for row in added {
            tables.push(toml_edit::ser::to_document(row).map_err(|_| invalid())?.into_table());
        }
    } else if let Some(array) = rows.as_array_mut() {
        array.retain(|value| value.as_inline_table().and_then(|row| id(row)).is_some_and(|key| after.contains_key(&key)));
        for value in array.iter_mut() {
            update(value.as_inline_table_mut().ok_or_else(invalid)?)?;
        }
        for row in added {
            array.push(toml_edit::ser::to_document(row).map_err(|_| invalid())?.into_table().into_inline_table());
        }
    } else {
        return Err(invalid());
    }
    Ok(toml.to_string())
}

pub fn plan_file_channel_models(reader: impl Read, mut writer: impl Write) -> io::Result<()> {
    let mut bytes = Vec::new();
    reader.take(MAX_REQUEST + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_REQUEST {
        return Err(invalid());
    }
    let input: Input = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    let document = release(sidecar(&input.model_config)?, input.local_http)?;
    let mut preview = crate::channel_http::preview_channel_model_plan(&document, input.plan)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err.to_string()))?;
    let candidate = preview.as_object_mut().and_then(|p| p.remove("candidate_routing"))
        .filter(|r| !r.is_null()).ok_or_else(|| invalid_config("select an existing configured channel"))?;
    // Preserve the explicit loopback parser and HTTPS parser used by startup.
    let routing = serde_json::from_value::<ModelRouting>(candidate).map_err(|_| invalid())?.resolve(input.local_http)?;
    let text = if routing == document.routing {
        input.model_config
    } else {
        write_routes(&input.model_config, &document.routing.routes, &routing.routes, routing.version)?
    };
    let checked = release(sidecar(&text)?, input.local_http)?;
    if checked.routing != routing || checked.pricing != document.pricing {
        return Err(invalid_config("rendered model sidecar differs from reviewed route plan"));
    }
    serde_json::to_writer(&mut writer, &json!({"model_config":text,"preview":preview}))?;
    writeln!(writer)
}
