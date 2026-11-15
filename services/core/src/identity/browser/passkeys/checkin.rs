//! A purpose-specific, mandatory-UV ceremony. Login compatibility is not used.
use super::*;

#[derive(Serialize, Deserialize)]
struct CheckinState {
    challenge: AuthenticationState,
    revisions: BTreeMap<String, i64>,
}

impl PasskeyConfig {
    pub(crate) async fn begin_checkin(
        &self,
        c: &mut PgConnection,
        user: i64,
    ) -> Result<Option<(Value, Value)>> {
        let rows = sqlx::query(
            "SELECT id,revision,passkey FROM core_passkeys.credentials WHERE user_id=$1 AND revoked_at IS NULL ORDER BY id LIMIT 10",
        )
        .bind(user)
        .fetch_all(c)
        .await?;
        if rows.is_empty() {
            return Ok(None);
        }
        let mut credentials = Vec::with_capacity(rows.len());
        let mut revisions = BTreeMap::new();
        for row in rows {
            let Json(key): Json<Passkey> = row.try_get("passkey")?;
            let mut credential = stored_credential(&key)?;
            credential.registration_policy = UserVerificationPolicy::Required;
            credentials.push(credential);
            revisions.insert(
                row.try_get::<i64, _>("id")?.to_string(),
                row.try_get("revision")?,
            );
        }
        let builder = self
            .source_policy_verifier
            .new_challenge_authenticate_builder(credentials, Some(UserVerificationPolicy::Required))
            .map_err(|_| IdentityError::Storage)?;
        let (options, challenge) = self
            .source_policy_verifier
            .generate_challenge_authenticate(builder)
            .map_err(|_| IdentityError::Storage)?;
        let mut options = serde_json::to_value(options).map_err(|_| IdentityError::Storage)?;
        options["publicKey"]["timeout"] = serde_json::json!(120_000);
        Ok(Some((
            options,
            serde_json::to_value(CheckinState {
                challenge,
                revisions,
            })
            .map_err(|_| IdentityError::Storage)?,
        )))
    }

    pub(crate) async fn finish_checkin(
        &self,
        c: &mut PgConnection,
        user: i64,
        state: Value,
        assertion: Value,
    ) -> Result<()> {
        let mut state: CheckinState =
            serde_json::from_value(state).map_err(|_| IdentityError::Storage)?;
        let reg: PublicKeyCredential =
            serde_json::from_value(assertion).map_err(|_| IdentityError::Unauthorized)?;
        let credential_id: &[u8] = reg.raw_id.as_ref();
        let row = sqlx::query(
            "SELECT k.id,k.revision,k.passkey,h.user_handle::text,l.user_handle AS legacy_user_handle FROM core_passkeys.credentials k JOIN core_passkeys.user_handles h ON h.user_id=k.user_id LEFT JOIN core_history.legacy_passkey_handles l ON l.credential_id=k.credential_id AND l.user_id=k.user_id WHERE k.user_id=$1 AND k.credential_id=$2 AND k.revoked_at IS NULL FOR UPDATE OF k NOWAIT",
        ).bind(user).bind(credential_id).fetch_optional(&mut *c).await?.ok_or(IdentityError::Unauthorized)?;
        let id: i64 = row.try_get("id")?;
        if state.revisions.get(&id.to_string()) != Some(&row.try_get::<i64, _>("revision")?) {
            return Err(IdentityError::Unauthorized);
        }
        if let Some(handle) = reg.get_user_unique_id().filter(|value| !value.is_empty()) {
            let legacy: Option<Vec<u8>> = row.try_get("legacy_user_handle")?;
            if !discoverable_handle_matches(
                Some(handle),
                &row.try_get::<String, _>("user_handle")?,
                legacy.as_deref(),
            ) {
                return Err(IdentityError::Unauthorized);
            }
        }
        let Json(mut key): Json<Passkey> = row.try_get("passkey")?;
        let mut credential = stored_credential(&key)?;
        credential.registration_policy = UserVerificationPolicy::Required;
        state.challenge.set_allowed_credentials(vec![credential]);
        let result = self
            .source_policy_verifier
            .authenticate_credential(&reg, &state.challenge)
            .map_err(|_| IdentityError::Unauthorized)?;
        if !result.user_verified() || key.update_credential(&result).is_none() {
            return Err(IdentityError::Unauthorized);
        }
        sqlx::query("UPDATE core_passkeys.credentials SET passkey=$2,last_used_at=clock_timestamp(),revision=revision+1 WHERE id=$1")
            .bind(id).bind(Json(key)).execute(c).await?;
        Ok(())
    }
}
