//! Exercise the real charging refund after an independently driven HTTP request.
use lmm_core::billing::{charging::Charging, transactional_ledger::TransactionalLedger};
use std::{error::Error, sync::Arc};

#[tokio::main(flavor = "multi_thread", worker_threads = 1)]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 4 {
        return Err("expected private URL file, request ID and refund credits".into());
    }
    let url = std::fs::read_to_string(&args[1])?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(url.trim())
        .await?;
    let charging = Charging::new(pool.clone(), Arc::new(TransactionalLedger));
    let charge = charging
        .refund(
            &args[2],
            "independent-finance-refund",
            args[3].parse()?,
            "synthetic independent usage refund",
        )
        .await?;
    println!(
        "{}",
        serde_json::json!({"request_id": args[2], "settled": charge.settled, "refunded": charge.refunded})
    );
    pool.close().await;
    Ok(())
}
