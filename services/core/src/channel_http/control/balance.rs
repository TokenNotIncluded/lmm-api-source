//! Published balance GET configuration and durable display-only observations.
use super::*;
use sqlx::{PgConnection, Row};

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BalanceProbe {
    pub path: String,
    pub json_pointer: String,
    pub unit_label: Option<String>,
}
impl BalanceProbe {
    /// Validate configuration at the publication boundary, including RFC 6901 escapes.
    pub(crate) fn validate(&self) -> Result<(), ManagementError> {
        let mut chars = self.json_pointer.chars();
        if !crate::model_http::channel_operations::valid_balance_path(&self.path)
            || chars.next() != Some('/')
            || self.unit_label.as_ref().is_some_and(|label| {
                label.chars().count() > 32 || label.chars().any(char::is_control)
            })
        {
            return Err(ManagementError::Invalid);
        }
        while let Some(c) = chars.next() {
            if c == '~' && !matches!(chars.next(), Some('0' | '1')) {
                return Err(ManagementError::Invalid);
            }
        }
        Ok(())
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
    probe: Option<BalanceProbe>,
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
    if let Some(probe) = &input.probe {
        probe.validate()?;
        // Reject secret-bearing configuration before it enters a publication or response.
        management
            .gateway
            .as_ref()
            .ok_or(Failure::Disabled)?
            .prepare_balance_probe(
                upstream,
                &probe.path,
                &probe.json_pointer,
                probe.unit_label.as_deref(),
            )
            .map_err(|_| ManagementError::Invalid)?;
        bindings.balance_probes.insert(id, probe.clone());
    } else {
        bindings.balance_probes.remove(&id);
    }
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
        json!({"success":true,"data":{"version":version,"probe":input.probe}}),
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
    let probe = bindings
        .balance_probes
        .get(&id)
        .ok_or(ObserveError::MissingProbe)?
        .clone();
    probe.validate()?;
    let request = gateway
        .prepare_balance_probe(
            &upstream,
            &probe.path,
            &probe.json_pointer,
            probe.unit_label.as_deref(),
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
        || bindings.balance_probes.get(&id) != Some(&probe)
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
