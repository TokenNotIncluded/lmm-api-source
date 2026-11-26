//! Shared health and fenced probe leases for an immutable routing configuration.
//! Network requests run outside database transactions.
use super::*;
use crate::routing::{
    CredentialRef, ProbeMode, ReleaseVersion, RoutingReliability, UpstreamConfig,
};
use sqlx::Row;
use std::collections::HashMap;

impl Bridge {
    async fn health_writer(
        &self,
        version: ReleaseVersion,
    ) -> Result<Option<sqlx::Transaction<'_, sqlx::Postgres>>, RelayError> {
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
        let active: Option<i64> = sqlx::query_scalar(
            "SELECT config_version FROM public.routing_active_release WHERE singleton",
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| RelayError::RouteUnavailable)?;
        if active.is_some_and(|v| v as u64 != version.routing) {
            return Ok(None);
        }
        Ok(Some(tx))
    }

    pub(super) async fn refresh_route_health(&self) -> Result<(), RelayError> {
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
        let rows = sqlx::query("SELECT channel_id,credential_id,credential_revision,disabled_scope FROM core_channels.route_health WHERE routing_version=$1 AND disabled_scope IS NOT NULL")
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

    pub(super) async fn disable_observed(
        &self,
        route: &crate::relay::Route,
        reason: &'static str,
        status: Option<u16>,
        elapsed_ms: u64,
    ) -> Result<(), RelayError> {
        let Some(attempt) = route.model_attempt.as_ref() else {
            return Ok(());
        };
        let Some(policy) = attempt.reliability().filter(|p| p.auto_disable) else {
            return Ok(());
        };
        let Some(mut tx) = self.health_writer(attempt.version()).await? else {
            return Ok(());
        };
        let key = attempt.credential_ref();
        let scope = if matches!(status, Some(401 | 403)) || reason == "error_keyword" {
            "credential"
        } else {
            "channel"
        };
        // A real failure invalidates any outstanding success, including a late result
        // from an older probe on the same revision.
        sqlx::query("INSERT INTO core_channels.route_health AS h (routing_version,channel_id,credential_id,credential_revision,disabled_scope,reason,status_code,latency_ms,next_probe_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,now()+make_interval(secs=>$9)) ON CONFLICT (routing_version,channel_id,credential_id,credential_revision) DO UPDATE SET disabled_scope=CASE WHEN h.disabled_scope='channel' THEN 'channel' ELSE EXCLUDED.disabled_scope END, reason=EXCLUDED.reason,status_code=EXCLUDED.status_code,latency_ms=EXCLUDED.latency_ms,next_probe_at=EXCLUDED.next_probe_at,generation=h.generation+1")
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
        self.bridge.refresh_route_health().await?;
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
        let Some(mut tx) = self.bridge.health_writer(version).await? else {
            return Ok(());
        };
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
        let result = self
            .probe_channel_snapshot_health(upstream, &policy.disable_keywords, &mut keyword_match)
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
            started.elapsed(),
            status,
            keyword_match,
        )
        .await
    }

    async fn finish_health_probe(
        &self,
        version: ReleaseVersion,
        policy: &RoutingReliability,
        channel: &UpstreamConfig,
        generation: i64,
        elapsed: Duration,
        status: Option<u16>,
        keyword_match: bool,
    ) -> Result<(), RelayError> {
        let slow = policy.too_slow(elapsed);
        let recover = status == Some(200) && !slow && policy.auto_recover;
        let reason = if policy.auto_disable
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
        let Some(mut tx) = self.bridge.health_writer(version).await? else {
            return Ok(());
        };
        sqlx::query(include_str!("channel_health_complete.sql"))
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
        tx.commit()
            .await
            .map_err(|_| RelayError::RouteUnavailable)?;
        self.bridge.refresh_route_health().await
    }
}
