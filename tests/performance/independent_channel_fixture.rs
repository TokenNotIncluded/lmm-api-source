//! Synthetic native identities and permission documents; never an HTTP oracle.
use lmm_core::{accounts::{Account,AccountKind},funding::FundingPolicy,identity::{self,IdentityStore}};
#[tokio::main(flavor="multi_thread",worker_threads=2)]
async fn main(){
    let url=std::fs::read_to_string(std::env::args().nth(1).unwrap()).unwrap();
    let pool=sqlx::postgres::PgPoolOptions::new().max_connections(2).connect(url.trim()).await.unwrap();
    lmm_core::core_schema::check(&pool).await.unwrap();
    let store=IdentityStore::from_pool(pool.clone());
    let mut sessions=serde_json::Map::new();
    for (user,level,name) in [(7,5,"admin"),(8,1,"ordinary"),(9,5,"admin_channel_deny"),(10,6,"root"),(11,6,"root_channel_deny"),(12,5,"revoked_admin")]{
        let credential=store.bootstrap_user(user,level).await.unwrap();
        if user==12{
            sqlx::query("UPDATE core_identity.credentials SET revoked_at=clock_timestamp() WHERE id=$1").bind(credential.id).execute(&pool).await.unwrap();
        }
        sessions.insert(name.into(),serde_json::json!({"user_id":user,"credential_id":credential.id,"secret":credential.secret}));
    }
    // Persist complete canonical nine-action documents. No decision helper is used as oracle.
    for user in [9i64,11]{
        let rows:Vec<(String,String)>=identity::Action::CATALOG.iter().map(|a|(a.catalog_name().unwrap().into(),if *a==identity::Action::ChannelRead{"deny"}else{"allow"}.into())).collect();
        let snapshot=identity::complete_policy_snapshot(user,1,&rows).unwrap();
        let payload=identity::canonical_policy_payload(&snapshot).unwrap();
        let hash=identity::policy_sha256(&snapshot).unwrap();
        let mut tx=pool.begin().await.unwrap();
        sqlx::query("INSERT INTO core_identity.permission_policies(user_id,revision,contract,sha256,canonical_payload,verified_complete) VALUES($1,1,$2,$3,$4,true)").bind(user).bind(identity::PERMISSIONS_CONTRACT).bind(hash.as_slice()).bind(payload).execute(&mut *tx).await.unwrap();
        for (action,effect) in &rows{
            sqlx::query("INSERT INTO core_identity.permission_policy_rows(user_id,action,effect) VALUES($1,$2,$3)").bind(user).bind(action).bind(effect).execute(&mut *tx).await.unwrap();
        }
        tx.commit().await.unwrap();
    }
    let admin=sessions["admin"]["secret"].as_str().unwrap();
    let mut keys=Vec::new();
    for group in ["allowed","秘密组"]{
        let key=store.issue_key(admin,Account{kind:AccountKind::Personal,id:7},FundingPolicy::new_personal_key(),3600).await.unwrap();
        sqlx::query("INSERT INTO core_identity.api_key_settings(credential_id,name,group_name) VALUES($1,'channel-independent',$2) ON CONFLICT(credential_id) DO UPDATE SET group_name=excluded.group_name").bind(key.id).bind(group).execute(&pool).await.unwrap();
        keys.push(serde_json::json!({"id":key.id,"group":group,"secret":key.secret}));
    }
    sqlx::raw_sql("INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) VALUES(7,'allowed',true,'independent-channel'),(7,'秘密组',true,'independent-channel')").execute(&pool).await.unwrap();
    println!("{}",serde_json::json!({"session":sessions["admin"]["secret"],"reader_session":sessions["ordinary"]["secret"],"sessions":sessions,"keys":keys}));
}
