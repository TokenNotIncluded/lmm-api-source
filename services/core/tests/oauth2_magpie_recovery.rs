//! Magpie uses its own managed refresh lineage, including lost-response recovery.
#[path = "common/model_fixture.rs"]
mod model_fixture;
#[expect(
    dead_code,
    reason = "Only managed refresh helpers are used by this target."
)]
#[path = "common/pi_oauth_fixture.rs"]
mod pi;
#[path = "common/runtime.rs"]
mod runtime;

use lmm_core::identity::grants::ManagedTokenForm;
use sqlx::PgPool;

#[sqlx::test(migrations = false)]
async fn magpie_retries_same_rotation_without_new_credentials_or_budget(pool: PgPool) {
    pi::with_magpie_fixture(pool, true, |f| async move {
        let installation = "e".repeat(64);
        let original = f
            .application_token("lmm-magpie", "magpie", 80, &installation)
            .await;
        let request = |installation: String, rotation: String| ManagedTokenForm {
            grant_type: "refresh_token".into(),
            client_id: "lmm-magpie".into(),
            resource: f.issuer.profile().resource().into(),
            installation_id: installation,
            code: None,
            redirect_uri: None,
            code_verifier: None,
            refresh_token: Some(original.refresh_token.clone()),
            scope: None,
            rotation_id: Some(rotation),
        };
        let rotate = || {
            request(installation.clone(), "f".repeat(64))
                .validate(f.issuer.profile())
                .unwrap()
        };
        let issued = f.issuer.token_managed(rotate()).await.unwrap();
        let recovered = f.issuer.token_managed(rotate()).await.unwrap();
        assert!(issued.access_token == recovered.access_token);
        assert!(issued.refresh_token == recovered.refresh_token);
        assert_eq!(issued.grant_id, original.grant_id);
        assert_eq!(recovered.grant_id, original.grant_id);
        let before = f
            .issuer
            .managed_application_budget(&issued.access_token)
            .await
            .unwrap();
        let charge = f
            .reserve(
                &recovered.access_token,
                &f.spec("magpie-recovered", 50),
                &[7; 32],
            )
            .await
            .unwrap();
        let after = f
            .issuer
            .managed_application_budget(&issued.access_token)
            .await
            .unwrap();
        assert_eq!(before["budget_credits"], after["budget_credits"]);
        assert_eq!(after["reserved_credits"], 50);
        assert_eq!(after["remaining_credits"], 30);
        f.charging.release(&charge.id, &[7; 32]).await.unwrap();
        // A new rotation on an already-consumed refresh is a genuine replay:
        // preserve the existing family-revocation policy after checking recovery.
        for (installation, rotation) in [
            ("2".repeat(64), "f".repeat(64)),
            (installation.clone(), "1".repeat(64)),
        ] {
            assert!(
                f.issuer
                    .token_managed(
                        request(installation, rotation)
                            .validate(f.issuer.profile())
                            .unwrap()
                    )
                    .await
                    .is_err()
            );
        }
        assert!(
            f.issuer
                .managed_application_budget(&issued.access_token)
                .await
                .is_err()
        );
    })
    .await;
}
