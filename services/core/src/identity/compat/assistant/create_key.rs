//! A credential is issued only inside the browser coordinator's transaction.
//! The model receives only the immutable preparation. Secret material lives in
//! an authenticated, encrypted, original-browser card and is consumed once.
use super::*;
use sqlx::postgres::PgRow;
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreationState {
    pub name: String,
    pub group: String,
    pub one_time_reveal: bool,
    pub expired_time: i64,
    pub remain_quota: i64,
    pub unlimited_quota: bool,
    pub model_limits_enabled: bool,
    pub cross_group_retry: bool,
    pub requires_two_factor: bool,
    pub mfa_revision: i64,
}
impl CreationState {
    fn input(&self) -> Result<DashboardKeyInput> {
        if !self.one_time_reveal
            || self.expired_time != -1
            || self.remain_quota != 0
            || !self.unlimited_quota
            || self.model_limits_enabled
            || self.cross_group_retry
        {
            return Err(IdentityError::Invalid);
        }
        Ok(DashboardKeyInput {
            id: None,
            name: Some(self.name.clone()),
            status: None,
            one_time_reveal: Some(true),
            remain_quota: Some(0),
            expired_time: Some(-1),
            unlimited_quota: Some(true),
            model_limits_enabled: Some(false),
            model_limits: Some(String::new()),
            allow_ips: Some(String::new()),
            group: Some(self.group.clone()),
            auto_groups: Some(Vec::new()),
            cross_group_retry: Some(false),
            group_warning_confirmations: None,
        })
    }
}
async fn creation_state(
    c: &mut PgConnection,
    actor: &Principal,
    name: &str,
    group: &str,
) -> Result<CreationState> {
    if name != name.trim() || group != group.trim() || group == "auto" {
        return Err(IdentityError::Invalid);
    }
    let (_, required, revision) = mfa::assistant_policy(c, actor.user_id).await?;
    let state = CreationState {
        name: name.into(),
        group: group.into(),
        one_time_reveal: true,
        expired_time: -1,
        remain_quota: 0,
        unlimited_quota: true,
        model_limits_enabled: false,
        cross_group_retry: false,
        requires_two_factor: required,
        mfa_revision: revision,
    };
    let marked = key_management(c, actor).await?;
    // Authoritative current routing and current dashboard validation only.
    // Historical L1 and unused legacy options are not creation authority.
    validate(c, actor.user_id, &state.input()?, None, marked).await?;
    Ok(state)
}
pub(crate) async fn prepare_create_key(
    c: &mut PgConnection,
    context: &VerifiedToolContext,
    name: &str,
    group: &str,
) -> Result<Draft> {
    let actor = original_actor(c, context).await?;
    Ok(Draft::CreateKey {
        before: creation_state(c, &actor, name, group).await?,
    })
}
fn card_aad(id: &str, credential: i64, user: i64) -> Vec<u8> {
    let mut aad = b"lmm:assistant-key-card:v1\0".to_vec();
    aad.extend_from_slice(id.as_bytes());
    aad.extend_from_slice(&credential.to_be_bytes());
    aad.extend_from_slice(&user.to_be_bytes());
    aad
}
fn encrypt_card(key: &[u8], id: &str, credential: i64, user: i64, secret: &str) -> Result<Vec<u8>> {
    let mut nonce = [0_u8; 12];
    SysRng
        .try_fill_bytes(&mut nonce)
        .map_err(|_| IdentityError::Storage)?;
    let encrypted = cipher(key)?
        .encrypt(
            &Nonce::from(nonce),
            Payload {
                msg: secret.as_bytes(),
                aad: &card_aad(id, credential, user),
            },
        )
        .map_err(|_| IdentityError::Storage)?;
    let mut output = nonce.to_vec();
    output.extend(encrypted);
    Ok(output)
}
fn decrypt_card(
    key: &[u8],
    id: &str,
    credential: i64,
    user: i64,
    encrypted: &[u8],
) -> Result<Zeroizing<String>> {
    if encrypted.len() < 44 {
        return Err(IdentityError::Storage);
    }
    let raw = Zeroizing::new(
        cipher(key)?
            .decrypt(
                &Nonce::try_from(&encrypted[..12]).map_err(|_| IdentityError::Storage)?,
                Payload {
                    msg: &encrypted[12..],
                    aad: &card_aad(id, credential, user),
                },
            )
            .map_err(|_| IdentityError::Storage)?,
    );
    String::from_utf8(raw.to_vec())
        .map(Zeroizing::new)
        .map_err(|_| IdentityError::Storage)
}
pub(super) async fn create(
    c: &mut PgConnection,
    store: &IdentityStore,
    actor: &Principal,
    before: &CreationState,
    confirmation_id: &str,
    code: Option<&str>,
) -> MfaResult<Value> {
    if &creation_state(c, actor, &before.name, &before.group).await? != before {
        return Err(IdentityError::Conflict.into());
    }
    let key = store
        .api_key_encryption_key
        .as_deref()
        .ok_or(IdentityError::Storage)?;
    // Validate configured key material before consuming a factor or issuing a key.
    let _ = cipher(key)?;
    store
        .verify_assistant_action_mfa(
            c,
            actor.user_id,
            before.requires_two_factor,
            before.mfa_revision,
            code,
        )
        .await?;
    let issued = store
        .create_dashboard_key_in_transaction(c, actor, &before.input()?)
        .await?;
    let encrypted = encrypt_card(
        key,
        confirmation_id,
        issued.credential_id,
        actor.user_id,
        issued.secret.as_str(),
    )?;
    sqlx::query(
        "WITH stamp AS MATERIALIZED (SELECT extract(epoch FROM clock_timestamp())::bigint AS now)
         INSERT INTO core_assistant_tools.key_creation_cards
          (confirmation_id,credential_id,ciphertext,created_at,expires_at)
         SELECT $1,$2,$3,stamp.now,stamp.now+600 FROM stamp",
    )
    .bind(confirmation_id)
    .bind(issued.credential_id)
    .bind(encrypted)
    .execute(c)
    .await?;
    Ok(json!({
        "id": issued.credential_id, "name": before.name, "group": before.group,
        "one_time_reveal": true, "expired_time": -1, "unlimited_quota": true,
        "model_limits_enabled": false, "cross_group_retry": false,
        "card_id": confirmation_id
    }))
}
async fn card_row(c: &mut PgConnection, context: &VerifiedToolContext, id: &str) -> Result<PgRow> {
    let actor = original_actor(c, context).await?;
    sqlx::query(
        "SELECT p.*,extract(epoch FROM clock_timestamp())::bigint AS now,
          k.digest,(k.revoked_at IS NOT NULL OR k.expires_at<=clock_timestamp()
            OR k.user_version<>u.auth_version OR NOT s.enabled) AS unavailable
         FROM core_assistant_tools.key_creation_cards p
         JOIN core_assistant_tools.confirmations q ON q.id=p.confirmation_id
         JOIN core_identity.credentials k ON k.id=p.credential_id
         JOIN core_identity.api_key_settings s ON s.credential_id=k.id
         JOIN core_identity.users u ON u.id=k.user_id
         WHERE q.id=$1 AND q.tool_name='request_create_key' AND q.state='approved'
          AND q.execution_receipt->>'action'='create'
          AND q.execution_receipt->'resource_id'=to_jsonb(k.id)
          AND q.actor_user_id=$2 AND q.account_id=$3 AND q.credential_id=$4
          AND k.kind='api_key' AND k.user_id=q.actor_user_id AND k.owner_account_id=q.account_id
         FOR UPDATE OF p FOR SHARE OF k,s,q",
    )
    .bind(id)
    .bind(actor.user_id)
    .bind(actor.owner_account_id)
    .bind(actor.credential_id)
    .fetch_optional(c)
    .await?
    .ok_or(IdentityError::Forbidden)
}
fn card_projection(row: &PgRow) -> Result<Value> {
    let state = if row.try_get::<Option<i64>, _>("consumed_at")?.is_some() {
        "consumed"
    } else if row.try_get::<i64, _>("expires_at")? <= row.try_get::<i64, _>("now")? {
        "expired"
    } else if row.try_get::<bool, _>("unavailable")? {
        "unavailable"
    } else {
        "ready"
    };
    Ok(json!({
        "state":state, "credential_id":row.try_get::<i64,_>("credential_id")?,
        "expires_at_unix":row.try_get::<i64,_>("expires_at")?, "one_time":true
    }))
}
pub(crate) async fn inspect_key_card(
    c: &mut PgConnection,
    context: &VerifiedToolContext,
    id: &str,
    nonce_sha256: &[u8],
) -> Result<Value> {
    if nonce_sha256.len() != 32 {
        return Err(IdentityError::Invalid);
    }
    let row = card_row(c, context, id).await?;
    let projection = card_projection(&row)?;
    if projection["state"] == "ready" {
        sqlx::query("UPDATE core_assistant_tools.key_creation_cards SET nonce_sha256=$2 WHERE confirmation_id=$1")
            .bind(id).bind(nonce_sha256).execute(c).await?;
    }
    Ok(projection)
}
pub(crate) async fn consume_key_card(
    c: &mut PgConnection,
    store: &IdentityStore,
    context: &VerifiedToolContext,
    id: &str,
    nonce_sha256: &[u8],
) -> Result<Value> {
    let row = card_row(c, context, id).await?;
    let projection = card_projection(&row)?;
    if projection["state"] != "ready" {
        // Includes a lost response after consumption. Never issue or reveal again.
        return Ok(json!({"ok":true,"key_card":projection}));
    }
    let nonce: Option<Vec<u8>> = row.try_get("nonce_sha256")?;
    if !nonce.is_some_and(|v| bool::from(v.as_slice().ct_eq(nonce_sha256))) {
        return Err(IdentityError::Forbidden);
    }
    let encrypted: Option<Vec<u8>> = row.try_get("ciphertext")?;
    let credential: i64 = row.try_get("credential_id")?;
    let key = store
        .api_key_encryption_key
        .as_deref()
        .ok_or(IdentityError::Storage)?;
    let secret = decrypt_card(
        key,
        id,
        credential,
        context.user_id(),
        encrypted.as_deref().ok_or(IdentityError::Storage)?,
    )?;
    if crate::identity::digest(secret.as_str())? != row.try_get::<Vec<u8>, _>("digest")? {
        return Err(IdentityError::Storage);
    }
    let changed = sqlx::query(
        "UPDATE core_assistant_tools.key_creation_cards SET ciphertext=NULL,
          consumed_at=extract(epoch FROM clock_timestamp())::bigint
         WHERE confirmation_id=$1 AND consumed_at IS NULL
          AND expires_at>extract(epoch FROM clock_timestamp())::bigint",
    )
    .bind(id)
    .execute(c)
    .await?
    .rows_affected();
    if changed != 1 {
        return Err(IdentityError::Conflict);
    }
    let mut consumed = projection;
    consumed["state"] = json!("consumed");
    Ok(json!({"ok":true,"key_card":consumed,"api_key":secret.as_str()}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_key_card_ciphertext_binds_confirmation_credential_actor_and_vault() {
        let id = "a".repeat(64);
        let key = [46; 32];
        let secret = "lmmk_synthetic_not_a_real_credential";
        let encrypted = encrypt_card(&key, &id, 19, 7, secret).unwrap();
        assert_eq!(
            decrypt_card(&key, &id, 19, 7, &encrypted).unwrap().as_str(),
            secret
        );
        assert!(decrypt_card(&key, &"b".repeat(64), 19, 7, &encrypted).is_err());
        assert!(decrypt_card(&key, &id, 20, 7, &encrypted).is_err());
        assert!(decrypt_card(&key, &id, 19, 8, &encrypted).is_err());
        assert!(decrypt_card(&[47; 32], &id, 19, 7, &encrypted).is_err());
        let mut corrupted = encrypted.clone();
        corrupted[15] ^= 1;
        assert!(decrypt_card(&key, &id, 19, 7, &corrupted).is_err());
        assert!(
            !encrypted
                .windows(secret.len())
                .any(|v| v == secret.as_bytes())
        );
    }
}
