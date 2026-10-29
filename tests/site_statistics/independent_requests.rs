//! Synthetic requests through real Identity and transactional Charging APIs.
//! This uses the public legacy Price contract, not the gateway trust-price path.
use lmm_core::{
    accounts::{Account, AccountKind},
    billing::{
        charging::{Charging, Outcome, Price, Request, Usage},
        transactional_ledger::TransactionalLedger,
    },
    funding::FundingPolicy,
    identity::IdentityStore,
};
use serde::Deserialize;
use std::{error::Error, sync::Arc};

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Reserve,
    Settle,
    Release,
}

#[derive(Deserialize)]
struct Input {
    operation: Operation,
    id: String,
    worker: [u8; 32],
    user_id: i64,
    session: Option<String>,
}

#[tokio::main(flavor = "multi_thread", worker_threads = 1)]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 3 {
        return Err("expected private URL and fixture input files".into());
    }
    let url = std::fs::read_to_string(&args[1])?;
    let input: Input = serde_json::from_slice(&std::fs::read(&args[2])?)?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(url.trim())
        .await?;
    let charging = Charging::new(pool.clone(), Arc::new(TransactionalLedger));
    let charge = match input.operation {
        Operation::Reserve => {
            let identity = IdentityStore::from_pool(pool.clone());
            let key = identity
                .issue_key(
                    input
                        .session
                        .as_deref()
                        .ok_or("reserve requires a real session")?,
                    Account {
                        kind: AccountKind::Personal,
                        id: input.user_id,
                    },
                    FundingPolicy::new_personal_key(),
                    3600,
                )
                .await?;
            sqlx::query("INSERT INTO core_identity.api_key_settings(credential_id,name,group_name) VALUES($1,'independent-seal-request','allowed')")
                .bind(key.id).execute(&pool).await?;
            let request = Request {
                id: input.id,
                fingerprint: [71; 32],
                price: Price {
                    user_trust: None,
                    image: None,
                    upstream_cost_group_ppm: None,
                    tiered: None,
                    version: "independent-legacy-price".into(),
                    model: "local-model".into(),
                    group: "allowed".into(),
                    input_per_million: 1_000_000,
                    cached_per_million: 500_000,
                    output_per_million: 2_000_000,
                },
                reserve: 30,
                lease_seconds: 300,
                maximum_seconds: 3600,
            };
            charging
                .reserve(&key.secret, &request, &input.worker)
                .await?
        }
        Operation::Settle => {
            if !charging.start(&input.id, &input.worker).await? {
                return Err("request did not enter streaming".into());
            }
            charging
                .settle(
                    &input.id,
                    &input.worker,
                    Usage {
                        input: 10,
                        cached: 0,
                        output: 5,
                        ..Usage::default()
                    },
                    Outcome::Completed,
                )
                .await?
        }
        Operation::Release => charging.release(&input.id, &input.worker).await?,
    };
    println!("{}", serde_json::to_string(&charge)?);
    pool.close().await;
    Ok(())
}
