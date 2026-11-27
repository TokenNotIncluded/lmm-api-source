use super::*;

#[sqlx::test(migrations = false)]
#[ignore = "requires packaged OpenCode V2 and loopback TLS on the hosted runner"]
async fn real_v2_oauth_session_settles_one_request(pool: PgPool) {
    provider_host::verify(
        pool,
        provider_host::Case {
            host: pi::CliHost::OpenCode,
            client: "lmm-opencode",
            script: "scripts/ci/opencode-v2-core.mjs",
            env: &["OPENCODE_V2_BIN", "OPENCODE_V2_PLUGIN"],
            receipt_env: "OPENCODE_V2_RECEIPT",
            scope: "OpenCode V2 host and packaged provider",
        },
    )
    .await;
}
