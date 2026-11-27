use super::*;

#[sqlx::test(migrations = false)]
#[ignore = "requires official CodeWhale and its pinned companion and loopback TLS on the hosted runner"]
async fn official_exec_settles_one_oauth_request(pool: PgPool) {
    provider_host::verify(
        pool,
        provider_host::Case {
            host: pi::CliHost::Codewhale,
            client: "lmm-codewhale",
            script: "scripts/ci/codewhale-core.mjs",
            env: &["LMM_CODEWHALE_BIN", "CODEWHALE_ADAPTER_CLI"],
            receipt_env: "CODEWHALE_CORE_RECEIPT",
            scope: "Official CodeWhale exec with its LMM companion",
        },
    )
    .await;
}
