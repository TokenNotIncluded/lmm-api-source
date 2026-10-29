//! Fixture identities and funds only; no performance rows or responses are seeded.
use lmm_core::{accounts::{Account,AccountKind},billing::{Credits,ledger::{Action,Ledger,Request}},funding::FundingPolicy,identity::IdentityStore};
#[tokio::main(flavor="multi_thread", worker_threads=2)]
async fn main() {
    let path=std::env::args().nth(1).unwrap();
    let url=std::fs::read_to_string(path).unwrap();
    let pool=sqlx::postgres::PgPoolOptions::new().max_connections(2).connect(url.trim()).await.unwrap();
    lmm_core::core_schema::check(&pool).await.unwrap();
    let identity=IdentityStore::from_pool(pool.clone());
    let session=identity.bootstrap_user(7,1).await.unwrap();
    let reader=identity.bootstrap_user(8,1).await.unwrap();
    let owner=identity.authorize(&session.secret).await.unwrap().owner_account_id;
    let ledger=Ledger::from_pool(pool.clone()); ledger.open_wallet(owner).await.unwrap();
    ledger.execute(&Request{scope:"independent-performance-local".into(),operation_key:"synthetic-funding".into(),actor_user_id:Some(7),reason:"Disposable independent local performance fixture".into(),action:Action::Credit{account_id:owner,amount_units:Credits::try_from(1_000_000_000i64).unwrap(),payment_reference:"synthetic:independent-performance".into()}}).await.unwrap();
    let mut keys=Vec::new();
    for group in ["allowed","秘密组"] {
        let key=identity.issue_key(&session.secret,Account{kind:AccountKind::Personal,id:7},FundingPolicy::new_personal_key(),3600).await.unwrap();
        sqlx::query("INSERT INTO core_identity.api_key_settings(credential_id,name,group_name) VALUES($1,'independent-local',$2) ON CONFLICT(credential_id) DO UPDATE SET group_name=excluded.group_name").bind(key.id).bind(group).execute(&pool).await.unwrap();
        keys.push(serde_json::json!({"id":key.id,"secret":key.secret,"group":group}));
    }
    sqlx::raw_sql("INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) VALUES (7,'allowed',true,'independent-local'),(7,'秘密组',true,'independent-local'),(8,'allowed',true,'independent-local') ON CONFLICT(user_id,group_name) DO UPDATE SET enabled=true").execute(&pool).await.unwrap();
    println!("{}",serde_json::json!({"session":session.secret,"reader_session":reader.secret,"owner_account":owner,"keys":keys}));
}
