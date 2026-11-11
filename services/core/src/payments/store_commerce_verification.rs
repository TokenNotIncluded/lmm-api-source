//! STORE goods evidence, deliberately separate from legacy/top-up application.
//! Root must declare this child module in payments.rs after Core62 freezes end.
use super::{Channel, Error, PaymentsStore, Result, providers, stripe::EventKind};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoreChannelScope {
    pub channel_id:i64,
    pub method:String,
    pub provider:String,
    pub merchant:String,
    pub environment:String,
    pub currency:String,
    pub store_id:String,
    pub product_id:String,
    pub payment_type:String,
    pub maximum_minor:i64,
    pub credits_per_minor:i64,
}

// No Deserialize or public constructor: a supplied receipt string cannot
// manufacture this proof. Only the existing Core signature verifiers can.
pub(crate) struct VerifiedStoreCharge {
    receipt:String,
    payment:String,
    charged_minor:i64,
    facts_sha256:Vec<u8>,
}
impl VerifiedStoreCharge {
    pub(crate) fn receipt(&self)->&str {&self.receipt}
    pub(crate) fn payment(&self)->&str {&self.payment}
    pub(crate) fn charged_minor(&self)->i64 {self.charged_minor}
    pub(crate) fn facts_sha256(&self)->&[u8] {&self.facts_sha256}
}
fn text<'a>(v:&'a Value,key:&str)->Result<&'a str> {
    v.get(key).and_then(Value::as_str).filter(|s|!s.is_empty() && s.len()<=256).ok_or(Error::Invalid)
}
fn short_id(s:&str,prefix:&str)->bool {
    s.starts_with(prefix) && s.len()>prefix.len() && s.len()<=128
        && s.bytes().all(|b|b.is_ascii_alphanumeric() || b==b'_')
}

impl PaymentsStore {
    pub(crate) fn store_channel_scope(&self,id:i64,method:&str)->Result<StoreChannelScope> {
        let c=self.channels.get(&id).ok_or(Error::Unavailable)?;
        let supported=match method {
            "external:waffo_pancake"|"platform:waffo_pancake"=>c.provider=="waffo_pancake"
                && !c.verification.store_id.is_empty() && !c.verification.product_id.is_empty(),
            "external:epay"=>c.provider=="epay",
            "platform:linuxdo"=>c.provider=="epay" && c.currency=="LDC",
            _=>false,
        };
        if !supported || c.credit_quote.is_some() {return Err(Error::Invalid)}
        Ok(StoreChannelScope{channel_id:id,method:method.into(),provider:c.provider.clone(),
            merchant:c.merchant.clone(),environment:c.environment.clone(),currency:c.currency.clone(),
            store_id:c.verification.store_id.clone(),product_id:c.verification.product_id.clone(),
            payment_type:c.verification.payment_type.clone(),maximum_minor:c.max_amount_minor,credits_per_minor:c.credits_per_minor})
    }

    fn bound_store_channel(&self,scope:&StoreChannelScope)->Result<Channel> {
        let mut current=self.store_channel_scope(scope.channel_id,&scope.method)?;
        // Admission limits can change without invalidating an existing paid
        // order. Identity, currency and merchant scope cannot be reassigned.
        current.maximum_minor=scope.maximum_minor;
        if &current!=scope {return Err(Error::Conflict)}
        let mut channel=self.channels.get(&scope.channel_id).ok_or(Error::Unavailable)?.clone();
        channel.max_amount_minor=scope.maximum_minor;Ok(channel)
    }

    pub(crate) fn verify_store_charge(&self,scope:&StoreChannelScope,order:&str,buyer:&str,
        quoted_minor:i64,payload:&[u8],signature:&[u8],now:i64)->Result<VerifiedStoreCharge> {
        if quoted_minor<=0 || payload.is_empty() || payload.len()>32768 || signature.is_empty() || signature.len()>2048 {
            return Err(Error::Invalid)
        }
        let channel=self.bound_store_channel(scope)?;
        let (receipt,payment,charged)=if scope.provider=="waffo_pancake" {
            let event=providers::pancake_envelope(payload,signature,&channel,now)?;
            let data=event.get("data").ok_or(Error::Invalid)?;
            let mode=if scope.environment=="live" {"prod"} else {"test"};
            if text(&event,"eventType")?!="order.completed" || text(&event,"mode")?!=mode
                || text(&event,"storeId")?!=scope.store_id
                || text(data,"orderMerchantExternalId")?!=order
                || text(data,"merchantProvidedBuyerIdentity")?!=buyer
                || text(data,"currency")?!=scope.currency
                || text(data,"paymentStatus")?!="succeeded"
                || data.get("orderStatus").filter(|v|!v.is_null()).is_some_and(|v|v.as_str()!=Some("completed"))
                || providers::decimal_minor(text(data,"amount")?)?!=quoted_minor {
                return Err(Error::Conflict)
            }
            // The checkout amount may exclude tax. Never substitute amount,
            // total, listPrice, a current FX value, or ORD for missing PAY.
            let charged=data.get("chargedAmount").and_then(Value::as_str).ok_or(Error::Unavailable)?;
            let payment=text(data,"paymentId")?;let receipt=text(data,"orderId")?;
            if !short_id(payment,"PAY") || !short_id(receipt,"ORD") {return Err(Error::Invalid)}
            if data.get("productId").filter(|v|!v.is_null()).is_some_and(|v|v.as_str()!=Some(scope.product_id.as_str())) {
                return Err(Error::Conflict)
            }
            (receipt.to_owned(),payment.to_owned(),providers::decimal_minor(charged)?)
        } else {
            let proof=providers::verify_epay(payload,signature,&channel,order,quoted_minor)?;
            if proof.kind!=EventKind::Credit {return Err(Error::Conflict)}
            (proof.transaction.clone(),proof.transaction,proof.amount_minor)
        };
        if charged<=0 || charged>scope.maximum_minor {return Err(Error::Invalid)}
        let facts=serde_json::to_vec(&serde_json::json!(["store-native-charge-v1",scope,order,buyer,
            receipt,payment,charged])).map_err(|_|Error::Storage)?;
        Ok(VerifiedStoreCharge{receipt,payment,charged_minor:charged,facts_sha256:Sha256::digest(facts).to_vec()})
    }
}
