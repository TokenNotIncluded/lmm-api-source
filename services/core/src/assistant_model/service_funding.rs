//! Operator-selected service funding; the authenticated conversation owner stays
//! the actor. This is independent from new-user promotional sponsorship.
use serde::{Deserialize, Serialize};
use sqlx::PgConnection;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ServiceFundingPolicy {
    pub payer_user_id: i64,
    pub payer_account_id: i64,
}

impl ServiceFundingPolicy {
    pub(crate) fn validate(&self) -> Result<(), super::Error> {
        if self.payer_user_id <= 0 || self.payer_account_id <= 0 {
            return Err(super::Error::Invalid);
        }
        Ok(())
    }

    /// Both grant creation and fresh reservation recheck the fixed service payer
    /// under the caller's transaction. IDs never come from an extension request.
    pub(crate) async fn admit(&self, c: &mut PgConnection) -> Result<(), super::Error> {
        self.validate()?;
        let payer: Option<i64> = sqlx::query_scalar(
            "SELECT a.id FROM core_identity.users u
             JOIN core_identity.accounts a ON a.id=u.personal_account_id
             WHERE u.id=$1 AND u.active AND u.platform_role='superadmin'
               AND a.id=$2 AND a.active AND a.kind='personal'
             FOR SHARE OF u,a",
        )
        .bind(self.payer_user_id)
        .bind(self.payer_account_id)
        .fetch_optional(c)
        .await?;
        if payer != Some(self.payer_account_id) {
            return Err(super::Error::Forbidden);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assistant_model::{Error, PersonalTextPolicy, SponsorPolicy};

    fn legacy_policy() -> PersonalTextPolicy {
        serde_json::from_str(
            r#"{"model":"synthetic-model","group":"synthetic-group","budget_credits":1000,
            "max_output_tokens":32,"ttl_seconds":300,"gift_tools":false,"sponsor":null}"#,
        )
        .unwrap()
    }

    #[test]
    fn service_funding_is_explicit_and_independent_from_gifts() {
        let mut policy = legacy_policy();
        assert_eq!(policy.service_funding, None);
        assert!(
            serde_json::to_value(&policy)
                .unwrap()
                .get("service_funding")
                .is_none()
        );
        policy.service_funding = Some(ServiceFundingPolicy {
            payer_user_id: 90001,
            payer_account_id: 2,
        });
        assert!(policy.validate().is_ok());
        let encoded = serde_json::to_value(&policy).unwrap();
        assert_eq!(encoded["gift_tools"], false);
        assert_eq!(encoded["service_funding"]["payer_account_id"], 2);
        assert_eq!(
            serde_json::from_value::<PersonalTextPolicy>(encoded).unwrap(),
            policy
        );
        policy.service_funding.as_mut().unwrap().payer_user_id = 0;
        assert_eq!(policy.validate(), Err(Error::Invalid));
    }

    #[test]
    fn service_funding_cannot_bypass_or_combine_new_user_sponsorship() {
        let mut policy = legacy_policy();
        policy.sponsor = Some(SponsorPolicy {
            policy_id: "synthetic-new-user".into(),
            treasury_account_id: 2,
            max_credits_per_user: 1000,
            global_budget_credits: 2000,
            max_calls_per_user: 8,
            expires_at_unix: 2000000000,
        });
        assert_eq!(
            policy.validate(),
            Err(Error::Invalid),
            "existing gift-tools guard"
        );
        policy.gift_tools = true;
        assert!(policy.validate().is_ok());
        policy.sponsor.as_mut().unwrap().max_calls_per_user = 9;
        assert_eq!(
            policy.validate(),
            Err(Error::Invalid),
            "existing eight-call guard"
        );
        policy.sponsor.as_mut().unwrap().max_calls_per_user = 8;
        policy.service_funding = Some(ServiceFundingPolicy {
            payer_user_id: 90001,
            payer_account_id: 2,
        });
        assert_eq!(
            policy.validate(),
            Err(Error::Invalid),
            "funding policies are exclusive"
        );
    }
}
