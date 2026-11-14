//! Published balance GET configuration and durable display-only observations.
use super::*;
use sqlx::{PgConnection, Row};

pub(super) mod legacy;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BalanceProbe {
    pub path: String,
    pub json_pointer: String,
    pub unit_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtract_json_pointer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require: Option<BalanceRequirement>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_all: Option<Vec<BalanceRequirement>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cny_per_usd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deepseek_currency_array: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai_subscription: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<BalanceAuth>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BalanceAuth {
    ApiKey,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BalanceRequirement {
    pub json_pointer: String,
    pub equals: Value,
}
impl BalanceProbe {
    /// Validate configuration at the publication boundary, including RFC 6901 escapes.
    pub(crate) fn validate(&self) -> Result<(), ManagementError> {
        fn valid_pointer(pointer: &str) -> bool {
            let mut chars = pointer.chars();
            if chars.next() != Some('/') {
                return false;
            }
            while let Some(c) = chars.next() {
                if c == '~' && !matches!(chars.next(), Some('0' | '1')) {
                    return false;
                }
            }
            true
        }
        if self
            .require_all
            .as_ref()
            .is_some_and(|requirements| requirements.is_empty() || requirements.len() > 8)
            || (self.openai_subscription == Some(true)
                && (self.path != "/v1/dashboard/billing/subscription"
                    || self.json_pointer != "/hard_limit_usd"
                    || self.unit_label.as_deref() != Some("USD")
                    || self.subtract_json_pointer.is_some()
                    || self.require.is_some()
                    || self.require_all.is_some()
                    || self.cny_per_usd.is_some()
                    || self.deepseek_currency_array == Some(true)
                    || self.auth.is_some()))
            || (self.deepseek_currency_array == Some(true)
                && (self.json_pointer != "/balance_infos"
                    || self.unit_label.as_deref() != Some("USD")
                    || self.subtract_json_pointer.is_some()))
            || self.cny_per_usd.as_ref().is_some_and(|rate| {
                rate.len() > 32
                    || rate.trim() != rate
                    || !rate.parse::<f64>().is_ok_and(|n| n.is_finite() && n > 0.0)
                    || self.unit_label.as_deref() != Some("USD")
                    || self.subtract_json_pointer.is_some()
            })
        {
            return Err(ManagementError::Invalid);
        }
        if !crate::model_http::channel_operations::valid_balance_path(&self.path)
            || !valid_pointer(&self.json_pointer)
            || self
                .subtract_json_pointer
                .as_deref()
                .is_some_and(|p| !valid_pointer(p))
            || self.requirements().into_iter().any(|(pointer, value)| {
                !valid_pointer(pointer)
                    || !match value {
                        Value::Bool(_) | Value::Number(_) => true,
                        Value::String(value) => {
                            value.len() <= 256 && !value.chars().any(char::is_control)
                        }
                        _ => false,
                    }
            })
            || self.unit_label.as_ref().is_some_and(|label| {
                label.chars().count() > 32 || label.chars().any(char::is_control)
            })
        {
            return Err(ManagementError::Invalid);
        }
        Ok(())
    }

    fn requirements(&self) -> Vec<(&str, &Value)> {
        self.require
            .iter()
            .chain(self.require_all.iter().flatten())
            .map(|r| (r.json_pointer.as_str(), &r.equals))
            .collect()
    }
}

/// Provider floats are represented as round-trippable strings in the Native API.
#[derive(Debug, Serialize)]
pub(in crate::channel_http) struct Observation {
    pub balance: Option<String>,
    pub unit_label: Option<String>,
    pub balance_updated_time: Option<i64>,
    pub kind: String,
    pub recorded_at: Option<i64>,
    pub source_channel_id: Option<u64>,
}

pub(super) async fn read_observation(
    c: &mut PgConnection,
    id: u64,
) -> Result<Observation, Failure> {
    let row = sqlx::query("SELECT balance,unit_label,floor(extract(epoch FROM observed_at))::bigint AS observed_at,kind,floor(extract(epoch FROM recorded_at))::bigint AS recorded_at,source_channel_id FROM core_channels.balance_observations WHERE channel_id=$1")
        .bind(i64::try_from(id).map_err(|_| ManagementError::Invalid)?)
        .fetch_optional(c).await.map_err(ManagementError::from)?;
    let Some(row) = row else {
        return Ok(Observation {
            balance: None,
            unit_label: None,
            balance_updated_time: None,
            kind: "unknown".to_owned(),
            recorded_at: None,
            source_channel_id: None,
        });
    };
    let balance: Option<f64> = row.try_get("balance").map_err(ManagementError::from)?;
    if balance.is_some_and(|balance| !balance.is_finite()) {
        return Err(ManagementError::Storage.into());
    }
    let source: Option<i64> = row
        .try_get("source_channel_id")
        .map_err(ManagementError::from)?;
    Ok(Observation {
        balance: balance.map(|balance| balance.to_string()),
        unit_label: row.try_get("unit_label").map_err(ManagementError::from)?,
        balance_updated_time: row.try_get("observed_at").map_err(ManagementError::from)?,
        kind: row.try_get("kind").map_err(ManagementError::from)?,
        recorded_at: row.try_get("recorded_at").map_err(ManagementError::from)?,
        source_channel_id: source
            .map(u64::try_from)
            .transpose()
            .map_err(|_| ManagementError::Storage)?,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::channel_http) struct BalanceConfigInput {
    expected_version: Version,
    #[serde(deserialize_with = "required_probe")]
    probe: Option<BalanceProbe>,
    #[serde(default)]
    legacy: Option<legacy::LegacyBalance>,
}

// Omission is not an instruction to clear configuration. Old clients already
// submit an explicit probe object or null.
fn required_probe<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<BalanceProbe>, D::Error> {
    Option::<BalanceProbe>::deserialize(deserializer)
}

pub(in crate::channel_http) async fn configure(
    State(state): State<ChannelState>,
    headers: HeaderMap,
    Path(id): Path<u64>,
    Json(input): Json<BalanceConfigInput>,
) -> Result<Json<Value>, Failure> {
    super::super::admit(&state, &headers).await?;
    let management = state.management.as_ref().ok_or(Failure::Disabled)?;
    let _guard = management.publication.lock().await;
    let (mut tx, principal) = management
        .begin(
            bearer(&headers)?,
            Action::ChannelSensitiveWrite,
            super::ReadView::Current,
        )
        .await?;
    let document = next_document(management, input.expected_version)?;
    let upstream = document
        .routing
        .upstreams
        .iter()
        .find(|upstream| upstream.id == id)
        .ok_or(Failure::NotFound)?;
    let mut bindings = management.current_bindings()?;
    if let Some(legacy) = &input.legacy {
        legacy.validate()?;
    }
    let automatic = legacy::resolve(None, input.legacy.as_ref(), upstream.protocol)?;
    // Validate both newly supplied plans, including an inactive automatic plan
    // hidden by a manual override, before persisting or returning configuration.
    for probe in input.probe.iter().chain(automatic.iter()) {
        probe.validate()?;
        // Reject secret-bearing configuration before it enters a publication or response.
        management
            .gateway
            .as_ref()
            .ok_or(Failure::Disabled)?
            .prepare_balance_probe(
                upstream,
                crate::model_http::BalanceProbeInput {
                    path: &probe.path,
                    pointer: &probe.json_pointer,
                    subtract_pointer: probe.subtract_json_pointer.as_deref(),
                    requirements: &probe.requirements(),
                    cny_per_usd: probe.cny_per_usd.as_deref(),
                    deepseek_currency_array: probe.deepseek_currency_array == Some(true),
                    openai_subscription: probe.openai_subscription == Some(true),
                    unit_label: probe.unit_label.as_deref(),
                    api_key_auth: matches!(probe.auth, Some(BalanceAuth::ApiKey)),
                },
            )
            .map_err(|_| ManagementError::Invalid)?;
    }
    if let Some(probe) = &input.probe {
        bindings.balance_probes.insert(id, probe.clone());
    } else {
        bindings.balance_probes.remove(&id);
        // An explicit clear disables both manual and automatic probing.
        bindings.legacy_balance.remove(&id);
    }
    if let Some(legacy) = &input.legacy {
        bindings.legacy_balance.insert(id, legacy.clone());
    }
    let saved_probe = bindings.balance_probes.get(&id).cloned();
    let saved_legacy = bindings.legacy_balance.get(&id).cloned();
    super::accounting::ensure_identities(management, &mut tx).await?;
    let version = management
        .publish_bindings(
            tx,
            &principal,
            input.expected_version,
            document,
            bindings,
            Vec::new(),
            PublicationChange::Normal {
                action: "channel.balance.configure",
                target: json!({"id":id}),
            },
        )
        .await?;
    Ok(Json(
        json!({"success":true,"data":{"version":version,"probe":saved_probe,"legacy":saved_legacy}}),
    ))
}

// Configuration of a probe necessarily creates a durable publication. Never use
// the bootstrap fallback for an observation or mix bindings from another version.
async fn durable_snapshot(c: &mut PgConnection) -> Result<(ReleaseDocument, Bindings), Failure> {
    let document: Option<Value> = sqlx::query_scalar("SELECT public.routing_control_read()")
        .fetch_one(&mut *c)
        .await
        .map_err(ManagementError::from)?;
    let document: ReleaseDocument =
        serde_json::from_value(document.ok_or(ManagementError::Conflict)?)
            .map_err(|_| ManagementError::Invalid)?;
    let bindings: Option<Value> = sqlx::query_scalar("SELECT public.routing_control_bindings($1)")
        .bind(document.routing.version as i64)
        .fetch_one(c)
        .await
        .map_err(ManagementError::from)?;
    let bindings = serde_json::from_value(bindings.ok_or(ManagementError::Conflict)?)
        .map_err(|_| ManagementError::Invalid)?;
    Ok((document, bindings))
}

enum ObserveError {
    Control(Failure),
    MissingProbe,
    Provider,
}
impl From<Failure> for ObserveError {
    fn from(error: Failure) -> Self {
        Self::Control(error)
    }
}
impl From<ManagementError> for ObserveError {
    fn from(error: ManagementError) -> Self {
        Self::Control(error.into())
    }
}
impl IntoResponse for ObserveError {
    fn into_response(self) -> Response {
        match self {
            Self::Control(error) => error.into_response(),
            Self::MissingProbe => error_response(
                StatusCode::NOT_IMPLEMENTED,
                "CHANNEL_BALANCE_PROBE_NOT_CONFIGURED",
                "Channel balance probe is not configured",
            ),
            Self::Provider => error_response(
                StatusCode::BAD_GATEWAY,
                "CHANNEL_BALANCE_PROVIDER_INVALID",
                "Provider balance probe failed",
            ),
        }
    }
}

async fn observe(
    state: &ChannelState,
    headers: &HeaderMap,
    id: u64,
) -> Result<(Observation, f64), ObserveError> {
    // This endpoint authorizes ChannelOperate, including on both sides of I/O.
    let management = state.management.as_ref().ok_or(Failure::Disabled)?;
    let secret = bearer(headers)?;
    let gateway = management.gateway.as_ref().ok_or(Failure::Disabled)?;
    let (mut tx, _) = management
        .begin(secret, Action::ChannelOperate, super::ReadView::Current)
        .await?;
    // Missing publication also means there cannot be a published balance probe.
    let active: Option<Value> = sqlx::query_scalar("SELECT public.routing_control_read()")
        .fetch_one(&mut *tx)
        .await
        .map_err(ManagementError::from)?;
    if active.is_none() {
        let document = management
            .routing
            .catalog_document()
            .map_err(|_| ManagementError::Storage)?;
        if !document
            .routing
            .upstreams
            .iter()
            .any(|upstream| upstream.id == id)
        {
            return Err(Failure::NotFound.into());
        }
        return Err(ObserveError::MissingProbe);
    }
    let (document, bindings) = durable_snapshot(&mut tx).await?;
    let upstream = document
        .routing
        .upstreams
        .iter()
        .find(|upstream| upstream.id == id)
        .ok_or(Failure::NotFound)?
        .clone();
    let probe = legacy::resolve(
        bindings.balance_probes.get(&id),
        bindings.legacy_balance.get(&id),
        upstream.protocol,
    )?
    .ok_or(ObserveError::MissingProbe)?;
    probe.validate()?;
    let request = gateway
        .prepare_balance_probe(
            &upstream,
            crate::model_http::BalanceProbeInput {
                path: &probe.path,
                pointer: &probe.json_pointer,
                subtract_pointer: probe.subtract_json_pointer.as_deref(),
                requirements: &probe.requirements(),
                cny_per_usd: probe.cny_per_usd.as_deref(),
                deepseek_currency_array: probe.deepseek_currency_array == Some(true),
                openai_subscription: probe.openai_subscription == Some(true),
                unit_label: probe.unit_label.as_deref(),
                api_key_auth: matches!(probe.auth, Some(BalanceAuth::ApiKey)),
            },
        )
        .map_err(|_| ObserveError::Provider)?;
    tx.commit()
        .await
        .map_err(|_| ManagementError::CommitUnknown)?;

    // No transaction, actor lock or publication mutex is held during provider I/O.
    let amount = gateway
        .probe_balance(request)
        .await
        .map_err(|_| ObserveError::Provider)?;

    let _publication = management.publication.lock().await;
    let (mut tx, principal) = management
        .begin(secret, Action::ChannelOperate, super::ReadView::Current)
        .await?;
    // Offline-installed SECURITY DEFINER helper fences publication tables with SHARE.
    // It holds no singleton row lock and also fences an absent active slot.
    sqlx::query("SELECT public.routing_control_balance_lock()")
        .execute(&mut *tx)
        .await
        .map_err(ManagementError::from)?;
    let (current, bindings) = durable_snapshot(&mut tx).await?;
    if Version::of(&current) != Version::of(&document)
        || current.routing.upstreams.iter().find(|item| item.id == id) != Some(&upstream)
        || legacy::resolve(
            bindings.balance_probes.get(&id),
            bindings.legacy_balance.get(&id),
            upstream.protocol,
        )? != Some(probe.clone())
    {
        return Err(ManagementError::Conflict.into());
    }
    sqlx::query("INSERT INTO core_channels.balance_observations(channel_id,balance,unit_label,observed_at,kind,credential_id,credential_revision,source_channel_id,recorded_at) VALUES($1,$2,$3,clock_timestamp(),'provider',$4,$5,NULL,clock_timestamp()) ON CONFLICT(channel_id) DO UPDATE SET balance=EXCLUDED.balance,unit_label=EXCLUDED.unit_label,observed_at=EXCLUDED.observed_at,kind=EXCLUDED.kind,credential_id=EXCLUDED.credential_id,credential_revision=EXCLUDED.credential_revision,source_channel_id=NULL,recorded_at=EXCLUDED.recorded_at")
        .bind(id as i64).bind(amount).bind(&probe.unit_label)
        .bind(upstream.credential.id as i64).bind(upstream.credential.revision as i64)
        .execute(&mut *tx).await.map_err(ManagementError::from)?;
    sqlx::query("INSERT INTO core_identity.audit(actor_user_id,credential_id,action,target) VALUES($1,$2,'channel.balance.observe',$3)")
        .bind(principal.user_id).bind(principal.credential_id)
        .bind(json!({"id":id,"version":Version::of(&current)}))
        .execute(&mut *tx).await.map_err(ManagementError::from)?;
    let observation = read_observation(&mut tx, id).await?;
    tx.commit()
        .await
        .map_err(|_| ManagementError::CommitUnknown)?;
    Ok((observation, amount))
}

pub(in crate::channel_http) async fn native(
    State(state): State<ChannelState>,
    headers: HeaderMap,
    Path(id): Path<u64>,
) -> Response {
    match observe(&state, &headers, id).await {
        Ok((observation, _)) => Json(json!({"success":true,"data":observation})).into_response(),
        Err(error) => error.into_response(),
    }
}

pub(in crate::channel_http) async fn legacy(
    State(state): State<ChannelState>,
    headers: HeaderMap,
    Path(id): Path<u64>,
) -> Response {
    match observe(&state, &headers, id).await {
        Ok((observation, amount)) => {
            Json(json!({"success":true,"data":amount,"observation":observation})).into_response()
        }
        Err(error) => error.into_response(),
    }
}

#[cfg(test)]
mod tests;
