//! Single registration of upgrade dispatch and suffix-specific behavior.
use super::*;
use futures_util::future::BoxFuture;

type Connected = for<'a> fn(
    &'a mut PgConnection,
    &'a UpgradeRequest,
) -> BoxFuture<'a, Result<UpgradeReport, UpgradeError>>;
type GrantRuntime =
    for<'a> fn(&'a mut PgConnection, &'a UpgradeRequest) -> BoxFuture<'a, Result<(), UpgradeError>>;
type ReceiptPrivileges = for<'a> fn(
    &'a suffix::Suffix,
    &'a mut PgConnection,
    &'a UpgradeRequest,
) -> BoxFuture<'a, Result<(), UpgradeError>>;
pub(super) struct UpgradeHooks {
    pub validate: fn(&UpgradeRequest) -> Result<(), UpgradeError>,
    pub authorize: fn() -> Result<(), UpgradeError>,
    pub connected: Connected,
    pub grant_runtime: GrantRuntime,
    pub receipt_privileges: ReceiptPrivileges,
}
#[derive(Clone, Copy)]
pub(super) enum TargetSource {
    FrozenJson,
    Captured(fn() -> Result<(String, String), UpgradeError>),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DataProjection {
    Exact,
    SubscriptionEpay,
}
#[derive(Clone, Copy)]
pub(super) struct SuffixPolicy {
    pub target: TargetSource,
    pub projection: DataProjection,
    #[cfg(test)]
    pub after_ddl: fn() -> Result<(), UpgradeError>,
}
impl SuffixPolicy {
    const DEFAULT: Self = Self {
        target: TargetSource::FrozenJson,
        projection: DataProjection::Exact,
        #[cfg(test)]
        after_ddl: || Ok(()),
    };
}
#[derive(Clone, Copy, Debug)]
pub(super) enum SuffixError {
    CompiledComponentsChanged,
    CompiledTargetChanged,
    UpgradeContractInvalid,
    CompiledPredecessorChanged,
    RuntimeReceiptConflict,
    ReceiptPrivilegesInvalid,
    OriginalReceiptAbsent,
    ReceiptOrCatalogConflict,
    PredecessorCatalogMismatch,
    PredecessorReceiptMismatch,
    PredecessorComponentsMismatch,
    PredecessorDataChanged,
    PreservationOrTargetMismatch,
}
pub(super) struct SuffixHooks {
    pub policy: SuffixPolicy,
    pub error: fn(SuffixError) -> &'static str,
}
pub(super) enum Engine {
    Legacy,
    Suffix(SuffixHooks),
}
pub(super) struct Registration {
    pub procedure: &'static str,
    pub hooks: UpgradeHooks,
    pub engine: Engine,
}
impl Registration {
    pub(super) fn suffix(&self) -> &SuffixHooks {
        match &self.engine {
            Engine::Suffix(hooks) => hooks,
            Engine::Legacy => unreachable!("legacy engine cannot initialize a suffix"),
        }
    }
}
pub(super) struct Spec<'a> {
    request: &'a UpgradeRequest,
    registration: &'static Registration,
}
impl<'a> Spec<'a> {
    pub(super) fn new(request: &'a UpgradeRequest) -> Self {
        Self {
            request,
            registration: lookup(&request.procedure),
        }
    }
    pub(super) fn validate(&self) -> Result<(), UpgradeError> {
        (self.registration.hooks.validate)(self.request)
    }
    pub(super) fn authorize_target(&self) -> Result<(), UpgradeError> {
        self.validate()?;
        (self.registration.hooks.authorize)()
    }
    pub(super) async fn connected(
        &self,
        c: &mut PgConnection,
    ) -> Result<UpgradeReport, UpgradeError> {
        (self.registration.hooks.connected)(c, self.request).await
    }
}
fn no_grants<'a>(
    _: &'a mut PgConnection,
    _: &'a UpgradeRequest,
) -> BoxFuture<'a, Result<(), UpgradeError>> {
    Box::pin(async { Ok(()) })
}
fn owner_receipt<'a>(
    spec: &'a suffix::Suffix,
    c: &'a mut PgConnection,
    r: &'a UpgradeRequest,
) -> BoxFuture<'a, Result<(), UpgradeError>> {
    Box::pin(spec.receipt_owner_only(c, r))
}
fn routing_installer() -> Result<(), UpgradeError> {
    let routing = COMPONENTS
        .iter()
        .find(|(name, _)| *name == "routing")
        .ok_or(UpgradeError::Storage)?
        .1;
    authorize_routing_target(routing)
}
fn authorize_trust() -> Result<(), UpgradeError> {
    routing_installer()?;
    compiled_predecessor().map(|_| ())
}
static TRUST: Registration = Registration {
    procedure: PROCEDURE,
    hooks: UpgradeHooks {
        validate: UpgradeRequest::validate_trust,
        authorize: authorize_trust,
        connected: |c, r| Box::pin(upgrade_connected(c, r)),
        grant_runtime: no_grants,
        receipt_privileges: owner_receipt,
    },
    engine: Engine::Legacy,
};
fn commerce_grants<'a>(
    c: &'a mut PgConnection,
    r: &'a UpgradeRequest,
) -> BoxFuture<'a, Result<(), UpgradeError>> {
    Box::pin(async move {
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "GRANT EXECUTE ON FUNCTION core_console.write_display_exchange_rate(text) TO {}",
            quote(&r.roles.runtime)
        )))
        .execute(c)
        .await?;
        Ok(())
    })
}
fn subscription_grants<'a>(
    c: &'a mut PgConnection,
    r: &'a UpgradeRequest,
) -> BoxFuture<'a, Result<(), UpgradeError>> {
    Box::pin(async move {
        let unexpected: bool = sqlx::query_scalar("SELECT EXISTS(SELECT FROM core_payments.provider_identifiers WHERE subscription_order_id IS NOT NULL)").fetch_one(&mut *c).await?;
        if unexpected {
            return Err(reject((SUBSCRIPTION_EPAY.suffix().error)(
                SuffixError::PredecessorDataChanged,
            )));
        }
        subscription_epay::grant_runtime(c, &r.roles.runtime).await
    })
}
#[cfg(test)]
fn continuity_after_ddl() -> Result<(), UpgradeError> {
    if responses_continuity::fail_after_ddl() {
        Err(reject("RESPONSES_CONTINUITY_INJECTED_AFTER_DDL"))
    } else {
        Ok(())
    }
}

// Each procedure appears exactly once below: phase hooks and suffix policies
// are generated together, including all static, non-allocating public errors.
macro_rules! registrations {
    (legacy { $( $id:ident => $module:ident, $authorize:expr; )* }
     suffix { $( $sid:ident => $smodule:ident, $prefix:literal, $grant:expr, $receipt:expr, $policy:expr; )* }) => {
        $(static $id: Registration = Registration {
            procedure: $module::PROCEDURE,
            hooks: UpgradeHooks { validate: $module::validate, authorize: $authorize,
                connected: |c,r| Box::pin($module::connected(c,r)), grant_runtime: no_grants, receipt_privileges: owner_receipt },
            engine: Engine::Legacy,
        };)*
        $(pub(super) static $sid: Registration = Registration {
            procedure: $smodule::PROCEDURE,
            hooks: UpgradeHooks { validate: $smodule::validate, authorize: $smodule::authorize_target,
                connected: |c,r| Box::pin($smodule::connected(c,r)), grant_runtime: $grant, receipt_privileges: $receipt },
            engine: Engine::Suffix(SuffixHooks { policy: $policy, error: |error| match error {
                SuffixError::CompiledComponentsChanged => concat!($prefix, "COMPILED_COMPONENTS_CHANGED"),
                SuffixError::CompiledTargetChanged => concat!($prefix, "COMPILED_TARGET_CHANGED"),
                SuffixError::UpgradeContractInvalid => concat!($prefix, "UPGRADE_CONTRACT_INVALID"),
                SuffixError::CompiledPredecessorChanged => concat!($prefix, "COMPILED_PREDECESSOR_CHANGED"),
                SuffixError::RuntimeReceiptConflict => concat!($prefix, "RUNTIME_RECEIPT_CONFLICT"),
                SuffixError::ReceiptPrivilegesInvalid => concat!($prefix, "RECEIPT_PRIVILEGES_INVALID"),
                SuffixError::OriginalReceiptAbsent => concat!($prefix, "ORIGINAL_RECEIPT_ABSENT"),
                SuffixError::ReceiptOrCatalogConflict => concat!($prefix, "RECEIPT_OR_CATALOG_CONFLICT"),
                SuffixError::PredecessorCatalogMismatch => concat!($prefix, "PREDECESSOR_CATALOG_MISMATCH"),
                SuffixError::PredecessorReceiptMismatch => concat!($prefix, "PREDECESSOR_RECEIPT_MISMATCH"),
                SuffixError::PredecessorComponentsMismatch => concat!($prefix, "PREDECESSOR_COMPONENTS_MISMATCH"),
                SuffixError::PredecessorDataChanged => concat!($prefix, "PREDECESSOR_DATA_CHANGED"),
                SuffixError::PreservationOrTargetMismatch => concat!($prefix, "PRESERVATION_OR_TARGET_MISMATCH"),
            } }),
        };)*
        static ENTRIES: &[&Registration] = &[&TRUST, $(&$id,)* $(&$sid,)*];
    };
}
registrations! {
    legacy {
        LEDGER => ledger, || { routing_installer()?; ledger::authorize_target() };
        ADDITIVE => additive, || { routing_installer()?; additive::authorize_target() };
        FINANCE => finance, finance::authorize_target;
        PAYMENT_GATEWAYS => payment_gateways, payment_gateways::authorize_target;
        REQUEST_OPERATIONS => request_operations, request_operations::authorize_target;
        COINBASE_PAYMENTS => coinbase_payments, coinbase_payments::authorize_target;
    }
    suffix {
        COMPLETION => completion, "COMPLETION_", no_grants, owner_receipt, SuffixPolicy::DEFAULT;
        ACCOUNT_APPEALS => account_appeals, "ACCOUNT_APPEALS_", no_grants, owner_receipt, SuffixPolicy::DEFAULT;
        CHANNEL_ACCOUNTING => channel_accounting, "CHANNEL_ACCOUNTING_", no_grants, owner_receipt, SuffixPolicy::DEFAULT;
        CHANNEL_PROJECTION => channel_projection, "CHANNEL_PROJECTION_", no_grants, owner_receipt, SuffixPolicy::DEFAULT;
        NAVIGATION_OPTIONS => navigation_options, "NAVIGATION_OPTIONS_", no_grants, owner_receipt, SuffixPolicy::DEFAULT;
        CHANNEL_OBSERVATIONS => channel_observations, "CHANNEL_OBSERVATIONS_", no_grants, owner_receipt, SuffixPolicy::DEFAULT;
        TRUST_POLICY => trust_policy, "TRUST_POLICY_EDIT_", no_grants, owner_receipt, SuffixPolicy::DEFAULT;
        COMMERCE_DISPLAY_RATE => commerce_display_rate, "COMMERCE_DISPLAY_RATE_", commerce_grants, owner_receipt, SuffixPolicy::DEFAULT;
        SUBSCRIPTION_EPAY => subscription_epay, "SUBSCRIPTION_EPAY_", subscription_grants, owner_receipt, SuffixPolicy { projection: DataProjection::SubscriptionEpay, ..SuffixPolicy::DEFAULT };
        CORE59_TO62 => core59_to62, "CORE62_", |c,r| Box::pin(core59_to62::grant_runtime(c, &r.roles.runtime)), |_,c,r| Box::pin(core59_to62::receipt_privileges(c, &r.roles.runtime)), SuffixPolicy { target: TargetSource::Captured(core59_to62::captured_target), ..SuffixPolicy::DEFAULT };
        PANCAKE_LANGUAGE => subscription_pancake_language, "PANCAKE_LANGUAGE_", no_grants, owner_receipt, SuffixPolicy::DEFAULT;
        PAYMENT_DISCOUNTS => payment_discounts, "PAYMENT_DISCOUNTS_", |c,r| Box::pin(payment_discounts::grant_runtime(c, &r.roles.runtime)), owner_receipt, SuffixPolicy::DEFAULT;
        PAYMENT_DISCOUNT_MANAGEMENT => payment_discount_management, "PAYMENT_DISCOUNT_MANAGEMENT_", |c,r| Box::pin(payment_discount_management::grant_runtime(c, &r.roles.runtime)), owner_receipt, SuffixPolicy::DEFAULT;
        RESPONSES_CONTINUITY => responses_continuity, "RESPONSES_CONTINUITY_", |c,r| Box::pin(responses_continuity::grant_runtime(c, &r.roles.runtime)), owner_receipt, SuffixPolicy { #[cfg(test)] after_ddl: continuity_after_ddl, ..SuffixPolicy::DEFAULT };
    }
}
fn lookup(procedure: &str) -> &'static Registration {
    // Preserve the historical unknown-procedure error from the original engine.
    ENTRIES
        .iter()
        .copied()
        .find(|entry| entry.procedure == procedure)
        .unwrap_or(&TRUST)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_procedure_set_and_single_registration() {
        let expected: std::collections::BTreeSet<_> = [
            "core-24-to-25-trust-pricing-v1",
            "core-26-ledger-escrow-submit-v1",
            "core-additive-components-v1",
            "core-site-finance-v1",
            "core-payment-gateways-v1",
            "core-request-operations-v1",
            "core-coinbase-payments-v1",
            "core-functional-completion-v1",
            "core-account-appeals-v1",
            "core-channel-accounting-v1",
            "core-channel-projection-v1",
            "core-navigation-options-v1",
            "core-channel-observations-v1",
            "core-trust-policy-edit-v1",
            "core-commerce-display-rate-v1",
            "core-subscription-epay-v1",
            "core-59-to-62-v1",
            "core-subscription-pancake-language-optional-v1",
            "core-payment-discounts-v1",
            "core-payment-discount-management-v1",
            "core-responses-continuity-v1",
        ]
        .into_iter()
        .collect();
        let actual: std::collections::BTreeSet<_> = ENTRIES.iter().map(|r| r.procedure).collect();
        assert_eq!(actual, expected);
        assert_eq!(ENTRIES.len(), expected.len(), "duplicate procedure");
        for entry in ENTRIES {
            assert!(std::ptr::eq(lookup(entry.procedure), *entry));
        }
        assert!(std::ptr::eq(lookup("unknown-procedure"), &TRUST));
    }
    #[test]
    fn phase_validation_hooks_preserve_module_entry_points() {
        type Validate = fn(&UpgradeRequest) -> Result<(), UpgradeError>;
        let expected: &[(&str, Validate)] = &[
            (PROCEDURE, UpgradeRequest::validate_trust),
            (ledger::PROCEDURE, ledger::validate),
            (additive::PROCEDURE, additive::validate),
            (finance::PROCEDURE, finance::validate),
            (payment_gateways::PROCEDURE, payment_gateways::validate),
            (request_operations::PROCEDURE, request_operations::validate),
            (coinbase_payments::PROCEDURE, coinbase_payments::validate),
            (completion::PROCEDURE, completion::validate),
            (account_appeals::PROCEDURE, account_appeals::validate),
            (channel_accounting::PROCEDURE, channel_accounting::validate),
            (channel_projection::PROCEDURE, channel_projection::validate),
            (navigation_options::PROCEDURE, navigation_options::validate),
            (
                channel_observations::PROCEDURE,
                channel_observations::validate,
            ),
            (trust_policy::PROCEDURE, trust_policy::validate),
            (
                commerce_display_rate::PROCEDURE,
                commerce_display_rate::validate,
            ),
            (subscription_epay::PROCEDURE, subscription_epay::validate),
            (core59_to62::PROCEDURE, core59_to62::validate),
            (
                subscription_pancake_language::PROCEDURE,
                subscription_pancake_language::validate,
            ),
            (payment_discounts::PROCEDURE, payment_discounts::validate),
            (
                payment_discount_management::PROCEDURE,
                payment_discount_management::validate,
            ),
            (
                responses_continuity::PROCEDURE,
                responses_continuity::validate,
            ),
        ];
        for (name, validate) in expected {
            assert!(
                std::ptr::fn_addr_eq(lookup(name).hooks.validate, *validate),
                "{name}"
            );
        }
    }
    #[test]
    fn all_suffix_errors_and_special_policies_are_frozen() {
        // Independent literal prefixes, not the registration macro's arguments.
        let expected = [
            ("core-functional-completion-v1", "COMPLETION_"),
            ("core-account-appeals-v1", "ACCOUNT_APPEALS_"),
            ("core-channel-accounting-v1", "CHANNEL_ACCOUNTING_"),
            ("core-channel-projection-v1", "CHANNEL_PROJECTION_"),
            ("core-navigation-options-v1", "NAVIGATION_OPTIONS_"),
            ("core-channel-observations-v1", "CHANNEL_OBSERVATIONS_"),
            ("core-trust-policy-edit-v1", "TRUST_POLICY_EDIT_"),
            ("core-commerce-display-rate-v1", "COMMERCE_DISPLAY_RATE_"),
            ("core-subscription-epay-v1", "SUBSCRIPTION_EPAY_"),
            ("core-59-to-62-v1", "CORE62_"),
            (
                "core-subscription-pancake-language-optional-v1",
                "PANCAKE_LANGUAGE_",
            ),
            ("core-payment-discounts-v1", "PAYMENT_DISCOUNTS_"),
            (
                "core-payment-discount-management-v1",
                "PAYMENT_DISCOUNT_MANAGEMENT_",
            ),
            ("core-responses-continuity-v1", "RESPONSES_CONTINUITY_"),
        ];
        let errors = [
            (
                SuffixError::CompiledComponentsChanged,
                "COMPILED_COMPONENTS_CHANGED",
            ),
            (
                SuffixError::CompiledTargetChanged,
                "COMPILED_TARGET_CHANGED",
            ),
            (
                SuffixError::UpgradeContractInvalid,
                "UPGRADE_CONTRACT_INVALID",
            ),
            (
                SuffixError::CompiledPredecessorChanged,
                "COMPILED_PREDECESSOR_CHANGED",
            ),
            (
                SuffixError::RuntimeReceiptConflict,
                "RUNTIME_RECEIPT_CONFLICT",
            ),
            (
                SuffixError::ReceiptPrivilegesInvalid,
                "RECEIPT_PRIVILEGES_INVALID",
            ),
            (
                SuffixError::OriginalReceiptAbsent,
                "ORIGINAL_RECEIPT_ABSENT",
            ),
            (
                SuffixError::ReceiptOrCatalogConflict,
                "RECEIPT_OR_CATALOG_CONFLICT",
            ),
            (
                SuffixError::PredecessorCatalogMismatch,
                "PREDECESSOR_CATALOG_MISMATCH",
            ),
            (
                SuffixError::PredecessorReceiptMismatch,
                "PREDECESSOR_RECEIPT_MISMATCH",
            ),
            (
                SuffixError::PredecessorComponentsMismatch,
                "PREDECESSOR_COMPONENTS_MISMATCH",
            ),
            (
                SuffixError::PredecessorDataChanged,
                "PREDECESSOR_DATA_CHANGED",
            ),
            (
                SuffixError::PreservationOrTargetMismatch,
                "PRESERVATION_OR_TARGET_MISMATCH",
            ),
        ];
        assert_eq!(
            ENTRIES
                .iter()
                .filter(|r| matches!(r.engine, Engine::Suffix(_)))
                .count(),
            14
        );
        for (name, prefix) in expected {
            let entry = lookup(name);
            let hooks = entry.suffix();
            for (error, tail) in errors {
                assert_eq!((hooks.error)(error), format!("{prefix}{tail}"));
            }
            assert_eq!(
                hooks.policy.projection,
                if name == "core-subscription-epay-v1" {
                    DataProjection::SubscriptionEpay
                } else {
                    DataProjection::Exact
                }
            );
            match hooks.policy.target {
                TargetSource::Captured(parse) => {
                    assert_eq!(name, "core-59-to-62-v1");
                    assert!(std::ptr::fn_addr_eq(
                        parse,
                        core59_to62::captured_target
                            as fn() -> Result<(String, String), UpgradeError>
                    ));
                }
                TargetSource::FrozenJson => assert_ne!(name, "core-59-to-62-v1"),
            }
            assert!(std::ptr::fn_addr_eq(
                entry.hooks.authorize,
                match name {
                    "core-functional-completion-v1" =>
                        completion::authorize_target as fn() -> Result<(), UpgradeError>,
                    "core-account-appeals-v1" =>
                        account_appeals::authorize_target as fn() -> Result<(), UpgradeError>,
                    "core-channel-accounting-v1" =>
                        channel_accounting::authorize_target as fn() -> Result<(), UpgradeError>,
                    "core-channel-projection-v1" =>
                        channel_projection::authorize_target as fn() -> Result<(), UpgradeError>,
                    "core-navigation-options-v1" =>
                        navigation_options::authorize_target as fn() -> Result<(), UpgradeError>,
                    "core-channel-observations-v1" =>
                        channel_observations::authorize_target as fn() -> Result<(), UpgradeError>,
                    "core-trust-policy-edit-v1" =>
                        trust_policy::authorize_target as fn() -> Result<(), UpgradeError>,
                    "core-commerce-display-rate-v1" =>
                        commerce_display_rate::authorize_target as fn() -> Result<(), UpgradeError>,
                    "core-subscription-epay-v1" =>
                        subscription_epay::authorize_target as fn() -> Result<(), UpgradeError>,
                    "core-59-to-62-v1" =>
                        core59_to62::authorize_target as fn() -> Result<(), UpgradeError>,
                    "core-subscription-pancake-language-optional-v1" =>
                        subscription_pancake_language::authorize_target
                            as fn() -> Result<(), UpgradeError>,
                    "core-payment-discounts-v1" =>
                        payment_discounts::authorize_target as fn() -> Result<(), UpgradeError>,
                    "core-payment-discount-management-v1" =>
                        payment_discount_management::authorize_target
                            as fn() -> Result<(), UpgradeError>,
                    "core-responses-continuity-v1" =>
                        responses_continuity::authorize_target as fn() -> Result<(), UpgradeError>,
                    _ => unreachable!(),
                }
            ));
        }
        for entry in ENTRIES
            .iter()
            .filter(|r| matches!(r.engine, Engine::Legacy))
        {
            assert!(std::ptr::fn_addr_eq(
                entry.hooks.grant_runtime,
                no_grants as GrantRuntime
            ));
        }
        assert!(std::ptr::fn_addr_eq(
            COMMERCE_DISPLAY_RATE.hooks.grant_runtime,
            commerce_grants as GrantRuntime
        ));
        assert!(std::ptr::fn_addr_eq(
            SUBSCRIPTION_EPAY.hooks.grant_runtime,
            subscription_grants as GrantRuntime
        ));
        assert!(std::ptr::fn_addr_eq(
            RESPONSES_CONTINUITY.suffix().policy.after_ddl,
            continuity_after_ddl as fn() -> Result<(), UpgradeError>
        ));
    }
    #[test]
    fn suffix_grant_receipt_and_fault_defaults_are_explicit() {
        let granted = [
            "core-commerce-display-rate-v1",
            "core-subscription-epay-v1",
            "core-59-to-62-v1",
            "core-payment-discounts-v1",
            "core-payment-discount-management-v1",
            "core-responses-continuity-v1",
        ];
        for entry in ENTRIES
            .iter()
            .filter(|r| matches!(r.engine, Engine::Suffix(_)))
        {
            assert_eq!(
                std::ptr::fn_addr_eq(entry.hooks.grant_runtime, no_grants as GrantRuntime),
                !granted.contains(&entry.procedure),
                "{} grants",
                entry.procedure,
            );
            assert_eq!(
                std::ptr::fn_addr_eq(
                    entry.hooks.receipt_privileges,
                    owner_receipt as ReceiptPrivileges
                ),
                entry.procedure != "core-59-to-62-v1",
                "{} receipt",
                entry.procedure,
            );
            assert_eq!(
                std::ptr::fn_addr_eq(
                    entry.suffix().policy.after_ddl,
                    SuffixPolicy::DEFAULT.after_ddl
                ),
                entry.procedure != "core-responses-continuity-v1",
                "{} fault",
                entry.procedure,
            );
        }
    }
    #[test]
    fn unknown_contract_still_rejects_before_target_authorization() {
        let request = UpgradeRequest {
            version: 0,
            procedure: "unknown-procedure".into(),
            upgrade_id: "invalid".into(),
            database: UpgradeDatabase {
                name: String::new(),
                oid: 0,
                system_identifier: String::new(),
                server_version_num: 0,
            },
            roles: UpgradeRoles {
                installer: String::new(),
                owner: String::new(),
                runtime: String::new(),
                extensions: String::new(),
            },
            predecessor_manifest_sha256: String::new(),
            predecessor_catalog_sha256: String::new(),
            successor_manifest_sha256: String::new(),
        };
        assert!(matches!(
            Spec::new(&request).authorize_target(),
            Err(UpgradeError::Rejected(
                "UPGRADE_DB_TARGET_OR_CONTRACT_INVALID"
            ))
        ));
    }
}
