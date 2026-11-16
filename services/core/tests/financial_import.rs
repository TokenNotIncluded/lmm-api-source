//! Exercise current Python against the sealed reviewed Core69 source fixture.
//! This does not admit current Core source for financial migration.
//! CI opts into its own network-isolated PostgreSQL fixture, never the app DB.
use std::{path::Path, process::Command};

#[test]
fn offline_financial_import_contracts_and_postgres() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut command = Command::new("python3");
    command
        .current_dir(root)
        .args(["-B", "scripts/run-reviewed-financial-tests.py"]);
    if std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true") {
        command.env("LMM_FINANCIAL_PG_TEST", "1");
    }
    assert!(
        command
            .status()
            .expect("run financial importer tests")
            .success(),
        "financial importer contract or PostgreSQL tests failed"
    );
}
