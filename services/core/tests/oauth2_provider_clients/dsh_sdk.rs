use super::*;

#[sqlx::test(migrations = false)]
#[ignore = "requires the pinned DSH Web host and its explicit fixture issuer SDK assembly"]
async fn sdk_oauth_session_settles_one_request(pool: PgPool) {
    provider_host::verify(
        pool,
        provider_host::Case {
            host: pi::CliHost::Dsh,
            client: "lmm-dsh",
            script: "scripts/dsh-core-host/driver.mjs",
            env: &["DSH_SDK_PACKAGE", "DSH_SDK_ASSEMBLY"],
            receipt_env: "DSH_SDK_RECEIPT",
            scope: "DSH SDK with explicit fixture issuer",
        },
    )
    .await;
}
