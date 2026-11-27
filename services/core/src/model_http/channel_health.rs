//! Shared health and fenced probe leases for an immutable routing configuration.
//! Network requests run outside database transactions.
use super::*;
use crate::routing::{
    CredentialRef, ProbeMode, ReleaseVersion, RoutingReliability, UpstreamConfig,
};
use sqlx::Row;
use std::collections::HashMap;

struct ProbeObservation {
    elapsed: Duration,
    status: Option<u16>,
    keyword_match: bool,
    retry_at: Option<std::time::SystemTime>,
}

impl Bridge {
    async fn health_writer(&self) -> Result<sqlx::Transaction<'_, sqlx::Postgres>, RelayError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|_| RelayError::RouteUnavailable)?;
        sqlx::query("SELECT pg_advisory_xact_lock_shared($1)")
            .bind(crate::routing::CHANNEL_HEALTH_PUBLICATION_LOCK)
            .execute(&mut *tx)
            .await
            .map_err(|_| RelayError::RouteUnavailable)?;
        Ok(tx)
    }

    pub(super) async fn refresh_route_health(&self) -> Result<(), RelayError> {
        // Admission on a bootstrap-only gateway needs no health or publication read.
        // The background reader still discovers the first remote policy.
        if self
            .routing
            .reliability_version()
            .map_err(|_| RelayError::RouteUnavailable)?
            .is_none()
        {
            return Ok(());
        }
        self.synchronize_route_health().await
    }

    async fn synchronize_route_health(&self) -> Result<(), RelayError> {
        if let Some(manager) = self
            .channel_manager
            .get()
            .and_then(std::sync::Weak::upgrade)
        {
            manager
                .synchronize_if_changed()
                .await
                .map_err(|_| RelayError::RouteUnavailable)?;
        }
        let Some(version) = self
            .routing
            .reliability_version()
            .map_err(|_| RelayError::RouteUnavailable)?
        else {
            return Ok(());
        };
        // A slower older snapshot must never overwrite a newer refresh on this process.
        let _guard = self.health_refresh.lock().await;
        let rows = sqlx::query("SELECT channel_id,credential_id,credential_revision,coalesce(disabled_scope,'channel') AS disabled_scope FROM core_channels.route_health WHERE routing_version=$1 AND (disabled_scope IS NOT NULL OR (reason='http_status' AND status_code=429 AND next_probe_at>statement_timestamp()))")
            .bind(version.routing as i64).fetch_all(&self.pool).await.map_err(|_| RelayError::RouteUnavailable)?;
        let mut disabled: HashMap<u64, HashMap<CredentialRef, bool>> = HashMap::new();
        for row in rows {
            let key = CredentialRef {
                id: row.get::<i64, _>("credential_id") as u64,
                revision: row.get::<i64, _>("credential_revision") as u64,
            };
            disabled
                .entry(row.get::<i64, _>("channel_id") as u64)
                .or_default()
                .insert(key, row.get::<String, _>("disabled_scope") == "channel");
        }
        self.routing
            .install_health(version, &disabled)
            .map_err(|_| RelayError::RouteUnavailable)
    }

    // Rate-limit observations update every key of a channel. A probe already
    // owns a row when it updates the other keys. Serialize these rare writers
    // by channel before taking any of the row locks.
    async fn lock_cooldown(tx: &mut sqlx::PgConnection, channel: u64) -> Result<(), RelayError> {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('lmm:channel-cooldown:' || $1::bigint::text, 0))")
            .bind(channel as i64).execute(tx).await.map_err(|_| RelayError::RouteUnavailable)?;
        Ok(())
    }

    async fn write_cooldown(
        &self,
        tx: &mut sqlx::PgConnection,
        version: ReleaseVersion,
        channel: u64,
        until: std::time::SystemTime,
    ) -> Result<(), RelayError> {
        let keys = self
            .routing
            .channel_health_keys(version, channel)
            .map_err(|_| RelayError::RouteUnavailable)?;
        let (ids, revisions): (Vec<i64>, Vec<i64>) = keys
            .iter()
            .map(|key| (key.id as i64, key.revision as i64))
            .unzip();
        sqlx::query(include_str!("channel_cooldown.sql"))
            .bind(version.routing as i64)
            .bind(channel as i64)
            .bind(ids)
            .bind(revisions)
            .bind(chrono::DateTime::<chrono::Utc>::from(until))
            .execute(tx)
            .await
            .map_err(|_| RelayError::RouteUnavailable)?;
        Ok(())
    }

    pub(super) async fn observe_cooldown(
        &self,
        attempt: &crate::routing::Attempt,
        until: std::time::SystemTime,
    ) -> Result<(), RelayError> {
        if attempt.reliability().is_none() {
            return Ok(());
        }
        let mut tx = self.health_writer().await?;
        Self::lock_cooldown(&mut tx, attempt.upstream_id()).await?;
        self.write_cooldown(&mut tx, attempt.version(), attempt.upstream_id(), until)
            .await?;
        tx.commit()
            .await
            .map_err(|_| RelayError::RouteUnavailable)?;
        self.refresh_route_health().await
    }

    pub(super) async fn disable_observed(
        &self,
        route: &crate::relay::Route,
        reason: &'static str,
        status: Option<u16>,
        elapsed_ms: u64,
        retry_at: Option<std::time::SystemTime>,
    ) -> Result<(), RelayError> {
        let Some(attempt) = route.model_attempt.as_ref() else {
            return Ok(());
        };
        if let Some(until) = retry_at {
            return self.observe_cooldown(attempt, until).await;
        }
        let Some(policy) = attempt.reliability().filter(|p| p.auto_disable) else {
            return Ok(());
        };
        let mut tx = self.health_writer().await?;
        let key = attempt.credential_ref();
        let scope = if matches!(status, Some(401 | 403)) || reason == "error_keyword" {
            "credential"
        } else {
            "channel"
        };
        // A real failure invalidates any outstanding success, including a late result
        // from an older probe on the same revision.
        sqlx::query("INSERT INTO core_channels.route_health AS h (routing_version,channel_id,credential_id,credential_revision,disabled_scope,reason,status_code,latency_ms,next_probe_at) SELECT $1,$2,$3,$4,$5,$6,$7,$8,now()+make_interval(secs=>$9) WHERE coalesce((public.routing_control_read() #>> '{routing,version}')::bigint,$1)=$1 ON CONFLICT (routing_version,channel_id,credential_id,credential_revision) DO UPDATE SET disabled_scope=CASE WHEN h.disabled_scope='channel' THEN 'channel' ELSE EXCLUDED.disabled_scope END, reason=EXCLUDED.reason,status_code=EXCLUDED.status_code,latency_ms=EXCLUDED.latency_ms,next_probe_at=CASE WHEN h.disabled_scope IS NOT NULL OR (h.reason='http_status' AND h.status_code=429) THEN greatest(h.next_probe_at,EXCLUDED.next_probe_at) ELSE EXCLUDED.next_probe_at END,generation=h.generation+1")
            .bind(attempt.version().routing as i64).bind(attempt.upstream_id() as i64).bind(key.id as i64).bind(key.revision as i64)
            .bind(scope).bind(reason).bind(status.map(|v|v as i16)).bind(elapsed_ms.min(i64::MAX as u64) as i64).bind(f64::from(policy.probe_interval_seconds))
            .execute(&mut *tx).await.map_err(|_|RelayError::RouteUnavailable)?;
        tx.commit()
            .await
            .map_err(|_| RelayError::RouteUnavailable)?;
        self.refresh_route_health().await
    }
}

impl ModelGateway {
    pub(crate) fn managed_channels(
        &self,
    ) -> Option<std::sync::Arc<crate::channel_http::ManagedChannels>> {
        self.bridge
            .channel_manager
            .get()
            .and_then(std::sync::Weak::upgrade)
    }

    pub(crate) async fn assistant_model_override(&self) -> Result<Option<String>, RelayError> {
        let Some(manager) = self.managed_channels() else {
            return Ok(None);
        };
        manager
            .synchronize_if_changed()
            .await
            .map_err(|_| RelayError::RouteUnavailable)?;
        manager
            .assistant_model()
            .map_err(|_| RelayError::RouteUnavailable)
    }

    pub(crate) fn bind_channel_management(
        &self,
        manager: std::sync::Weak<crate::channel_http::ManagedChannels>,
    ) {
        let _ = self.bridge.channel_manager.set(manager);
    }
    /// One bounded worker per process. The database owns eligibility and the
    /// lease, so two instances can share work without probing a key twice.
    pub async fn run_route_health(&self, mut stop: tokio::sync::watch::Receiver<bool>) {
        let mut ticks = tokio::time::interval(Duration::from_secs(1));
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = stop.changed() => return,
                _ = ticks.tick() => {}
            }
            if *stop.borrow() {
                return;
            }
            tokio::select! {
                _ = stop.changed() => return,
                result = self.probe_due_health() => if result.is_err() { eprintln!("channel health refresh failed"); }
            }
        }
    }

    async fn probe_due_health(&self) -> Result<(), RelayError> {
        self.bridge.synchronize_route_health().await?;
        let Some((version, policy, channels)) = self
            .bridge
            .routing
            .reliability_probes()
            .map_err(|_| RelayError::RouteUnavailable)?
        else {
            return Ok(());
        };
        if channels.is_empty() {
            return Ok(());
        }
        let ids: Vec<i64> = channels.iter().map(|u| u.id as i64).collect();
        let keys: Vec<i64> = channels.iter().map(|u| u.credential.id as i64).collect();
        let revs: Vec<i64> = channels
            .iter()
            .map(|u| u.credential.revision as i64)
            .collect();
        // Lease expiry is longer than the network timeout. An old owner whose lease
        // expired can never complete another owner's generation.
        let lease_seconds = self.limits.total_timeout.as_secs_f64() + 30.0;
        let mut tx = self.bridge.health_writer().await?;
        let row = sqlx::query(include_str!("channel_health_claim.sql"))
            .bind(version.routing as i64)
            .bind(ids)
            .bind(keys)
            .bind(revs)
            .bind(policy.probe_mode == ProbeMode::ScheduledAll)
            .bind(lease_seconds)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|_| RelayError::RouteUnavailable)?;
        tx.commit()
            .await
            .map_err(|_| RelayError::RouteUnavailable)?;
        let Some(row) = row else { return Ok(()) };
        let id = row.get::<i64, _>("channel_id") as u64;
        let key = CredentialRef {
            id: row.get::<i64, _>("credential_id") as u64,
            revision: row.get::<i64, _>("credential_revision") as u64,
        };
        let upstream = channels
            .iter()
            .find(|u| u.id == id && u.credential == key)
            .ok_or(RelayError::RouteUnavailable)?;
        let started = std::time::Instant::now();
        let mut keyword_match = false;
        let mut retry_at = None;
        let result = self
            .probe_channel_snapshot_health(
                upstream,
                &policy.disable_keywords,
                &mut keyword_match,
                &mut retry_at,
            )
            .await;
        let status = match &result {
            Ok(_) => Some(200),
            Err(RelayError::UpstreamStatus(code)) => Some(*code),
            _ => None,
        };
        self.finish_health_probe(
            version,
            &policy,
            upstream,
            row.get("generation"),
            ProbeObservation {
                elapsed: started.elapsed(),
                status,
                keyword_match,
                retry_at,
            },
        )
        .await
    }

    async fn finish_health_probe(
        &self,
        version: ReleaseVersion,
        policy: &RoutingReliability,
        channel: &UpstreamConfig,
        generation: i64,
        observation: ProbeObservation,
    ) -> Result<(), RelayError> {
        let ProbeObservation {
            elapsed,
            status,
            keyword_match,
            retry_at,
        } = observation;
        let slow = policy.too_slow(elapsed);
        let recover = status == Some(200) && !slow && policy.auto_recover;
        let reason = if retry_at.is_some() {
            None
        } else if policy.auto_disable
            && status.is_some_and(|s| s != 200 && policy.disable_status_codes.includes(s))
        {
            Some("http_status")
        } else if policy.auto_disable && keyword_match {
            Some("error_keyword")
        } else if policy.auto_disable && slow && status.is_some() {
            Some("slow_response")
        } else {
            None
        };
        let scope = reason.map(|r| {
            if matches!(status, Some(401 | 403)) || r == "error_keyword" {
                "credential"
            } else {
                "channel"
            }
        });
        let mut tx = self.bridge.health_writer().await?;
        if retry_at.is_some() {
            Bridge::lock_cooldown(&mut tx, channel.id).await?;
        }
        let update = sqlx::query(include_str!("channel_health_complete.sql"))
            .bind(version.routing as i64)
            .bind(channel.id as i64)
            .bind(channel.credential.id as i64)
            .bind(channel.credential.revision as i64)
            .bind(generation)
            .bind(elapsed.as_millis().min(i64::MAX as u128) as i64)
            .bind(f64::from(policy.probe_interval_seconds))
            .bind(recover)
            .bind(scope)
            .bind(reason)
            .bind(status.map(|s| s as i16))
            .execute(&mut *tx)
            .await
            .map_err(|_| RelayError::RouteUnavailable)?;
        if let Some(until) = retry_at.filter(|_| update.rows_affected() == 1) {
            self.bridge
                .write_cooldown(&mut tx, version, channel.id, until)
                .await?;
        }
        tx.commit()
            .await
            .map_err(|_| RelayError::RouteUnavailable)?;
        self.bridge.refresh_route_health().await
    }
}
