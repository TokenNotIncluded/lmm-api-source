//! External GOODS obligations, original native receipts and a durable dispatch
//! fence. No top-up application and no caller-asserted verification bit.
//! Not registered in Core62: root owns the module declarations/RPC/schema wiring.
use super::{Account, CommerceStore, CredentialKind, Error, Result, authority, hash_id, hex, id, native};
use crate::payments::{PaymentsStore, store_commerce_verification::StoreChannelScope};
use serde::{Deserialize,Serialize};
use sha2::{Digest,Sha256};
use sqlx::{PgPool,Postgres,Row,Transaction,types::Json};

pub const SCHEMA:&str=include_str!("../../schema/commerce_external_payments.sql");

#[derive(Clone,Debug,Deserialize,Eq,PartialEq,Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalOrder {
    pub shop_id:String,
    pub order_id:String,
    pub buyer:Account,
    pub seller:Account,
    pub quantity:i64,
    pub unit_credits:i64,
    pub principal_credits:i64,
    pub quoted_minor:i64,
    pub currency:String,
    pub quote_version:String,
    pub channel_id:i64,
    pub method:String,
}
#[derive(Clone,Debug,Deserialize,Eq,PartialEq,Serialize)]
pub struct OrderReceipt {
    pub id:String,
    pub command_hash:String,
    pub provider_order_reference:String,
    pub provider_buyer_reference:String,
}
#[derive(Clone,Debug,Deserialize,Eq,PartialEq,Serialize,sqlx::FromRow)]
pub struct BasisReceipt {
    pub id:String,
    pub receipt_reference:String,
    pub payment_reference:String,
    pub charged_minor:i64,
    pub currency:String,
    pub facts_sha256:Vec<u8>,
}
#[derive(Clone,Debug,Deserialize,Eq,PartialEq,Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalRefund {
    pub order:ExternalOrder,
    pub basis_id:String,
    pub request_key:String,
    pub mode:String,
    pub quantity:i64,
    pub amount_credits:i64,
    pub amount_minor:i64,
    pub items_digest:String,
}
#[derive(Clone,Debug,Deserialize,Eq,PartialEq,Serialize,sqlx::FromRow)]
pub struct RefundReceipt {
    pub id:String,
    pub basis_id:String,
    pub command_sha256:Vec<u8>,
    pub credits:i64,
    pub native_minor:i64,
    pub retired_quantity:i64,
    pub state:String,
    pub submit_count:i16,
    pub dispatch_fence:Option<String>,
    pub provider_execution_receipt:Option<String>,
    pub settlement_receipt:Option<String>,
}

#[derive(Clone)]
pub struct ExternalCommerce {
    commerce:CommerceStore,
    payments:PaymentsStore,
    principal:String,
}
// Internal, non-cloneable, non-serializable one-shot authority. Future Core
// dispatch consumes it locally; no ClaimDispatch RPC may return it to Go.
pub(crate) struct DispatchPermit {refund_id:String,fence:String}
fn payment_error(error:crate::payments::Error)->Error {
    use crate::payments::Error as P;
    match error {P::Invalid=>Error::Invalid,P::Denied=>Error::Forbidden,P::Conflict=>Error::Conflict,
        P::Unavailable=>Error::Pending,P::NotFound=>Error::Missing,P::Storage=>Error::Storage}
}
fn digest<T:Serialize>(value:&T)->Result<Vec<u8>> {
    Ok(Sha256::digest(serde_json::to_vec(value).map_err(|_|Error::Invalid)?).to_vec())
}
fn order_id(order:&ExternalOrder)->String {id(&serde_json::json!(["external-store-order-v1",order.shop_id,order.order_id]))}
fn text(s:&str,max:usize)->bool {!s.is_empty() && s.len()<=max && !s.chars().any(char::is_control)}
fn validate_order(o:&ExternalOrder)->Result<()> {
    if !hash_id(&o.shop_id) || !hash_id(&o.order_id) || !o.buyer.is_valid() || !o.seller.is_valid()
        || o.quantity<=0 || o.quantity>1000 || o.unit_credits<=0
        || o.unit_credits.checked_mul(o.quantity)!=Some(o.principal_credits)
        || o.quoted_minor<=0 || !text(&o.quote_version,128) || o.channel_id<=0 {
        return Err(Error::Invalid)
    } Ok(())
}
async fn lock_order(tx:&mut Transaction<'_,Postgres>,key:&str)->Result<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,38590272))").bind(key).execute(&mut **tx).await?;Ok(())
}
async fn saved_refund(tx:&mut Transaction<'_,Postgres>,key:&str)->Result<RefundReceipt> {
    Ok(sqlx::query_as("SELECT id,basis_id,command_sha256,credits,native_minor,retired_quantity,state,submit_count,dispatch_fence,provider_execution_receipt,settlement_receipt FROM core_commerce_external.refunds WHERE id=$1")
        .bind(key).fetch_one(&mut **tx).await?)
}

// Exact cumulative integer planning. Native charge (including tax) and credits
// are separate ceilings; neither is recalculated from current FX or catalogue.
#[derive(Default)]
struct Allocated {credits:i64,minor:i64,quantity:i64,quantity_minor:i64,retired:i64}
fn fraction(total:i64,part:i64,whole:i64)->Result<i64> {
    if total<0 || part<0 || whole<=0 || part>whole {return Err(Error::Conflict)}
    i64::try_from(i128::from(total)*i128::from(part)/i128::from(whole)).map_err(|_|Error::Invalid)
}
fn plan_refund(r:&ExternalRefund,charged:i64,a:&Allocated)->Result<(i64,i64,i64)> {
    let total=r.order.principal_credits;let qty=r.order.quantity;
    if a.credits<0 || a.credits>total || a.minor<0 || a.minor>charged || a.retired<0 || a.retired>qty
        || a.quantity<0 || a.quantity>qty || a.quantity_minor<0 || a.quantity_minor>charged {return Err(Error::Conflict)}
    let (credits,minor,retired)=match r.mode.as_str() {
        "full" if r.quantity==0 && r.amount_credits==0 && r.amount_minor==0=>(total-a.credits,charged-a.minor,qty-a.retired),
        "quantity" if r.quantity>0 && r.quantity<=qty-a.retired && r.amount_credits==0 && r.amount_minor==0=> {
            let credits=r.order.unit_credits.checked_mul(r.quantity).ok_or(Error::Invalid)?;
            let q=a.quantity.checked_add(r.quantity).filter(|q|*q<=qty).ok_or(Error::Conflict)?;
            (credits,fraction(charged,q,qty)?.checked_sub(a.quantity_minor).ok_or(Error::Conflict)?,r.quantity)
        }
        "amount" if r.quantity==0 && r.amount_credits>0 && r.amount_minor==0=> {
            let cumulative=a.credits.checked_add(r.amount_credits).ok_or(Error::Invalid)?;
            (r.amount_credits,fraction(charged,cumulative,total)?.checked_sub(a.minor).ok_or(Error::Conflict)?,0)
        }
        "amount" if r.quantity==0 && r.amount_minor>0 && r.amount_credits==0=> {
            let cumulative=a.minor.checked_add(r.amount_minor).ok_or(Error::Invalid)?;
            (fraction(total,cumulative,charged)?.checked_sub(a.credits).ok_or(Error::Conflict)?,r.amount_minor,0)
        }
        _=>return Err(Error::Invalid),
    };
    if credits<=0 || minor<=0 || credits>total-a.credits || minor>charged-a.minor {return Err(Error::Conflict)}
    // Both original ceilings must exhaust together. Never consume the final
    // native cent while leaving sellable unpaid goods or credit principal.
    if (credits==total-a.credits)!=(minor==charged-a.minor) {return Err(Error::Conflict)}
    let retired=if credits==total-a.credits {qty-a.retired} else {retired};
    Ok((credits,minor,retired))
}

impl ExternalCommerce {
    pub async fn prepare_refund(&self,secret:&str,request:&ExternalRefund)->Result<RefundReceipt> {
        validate_order(&request.order)?;
        if !hash_id(&request.basis_id) || !hash_id(&request.items_digest) || !text(&request.request_key,64)
            || !request.request_key.bytes().all(|b|b.is_ascii_alphanumeric() || b==b'_' || b==b'-') {
            return Err(Error::Invalid)
        }
        let key=order_id(&request.order);
        let refund=id(&serde_json::json!(["store-external-refund-v1",key,request.request_key]));
        let hash=digest(request)?;
        let mut tx=self.commerce.begin().await?;lock_order(&mut tx,&key).await?;
        let actor=crate::identity::authorize_in_transaction(&mut tx,secret).await?;
        let seller=native(&mut tx,request.order.seller,true).await?;
        authority(&mut tx,&actor,request.order.seller,seller,false).await?;
        let row=sqlx::query("SELECT service_principal,command_sha256,seller_account_id FROM core_commerce_external.orders WHERE id=$1 FOR SHARE")
            .bind(&key).fetch_optional(&mut *tx).await?.ok_or(Error::Missing)?;
        if row.try_get::<String,_>("service_principal")?!=self.principal || row.try_get::<Vec<u8>,_>("command_sha256")?!=digest(&request.order)?
            || row.try_get::<i64,_>("seller_account_id")?!=seller {return Err(Error::Conflict)}
        if let Some(saved)=sqlx::query("SELECT command_sha256 FROM core_commerce_external.refunds WHERE id=$1").bind(&refund).fetch_optional(&mut *tx).await? {
            if saved.try_get::<Vec<u8>,_>("command_sha256")?!=hash {return Err(Error::Conflict)}
            let receipt=saved_refund(&mut tx,&refund).await?;tx.commit().await?;return Ok(receipt)
        }
        let basis=Self::basis(&mut tx,&key).await?;
        if basis.id!=request.basis_id {return Err(Error::Conflict)}
        // LinuxDO's documented native protocol is full only; EPay has no
        // universal refund submit API, even if its currency happens to be LDC.
        if request.order.method=="platform:linuxdo" && request.mode!="full" {return Err(Error::Invalid)}
        let rows=sqlx::query("SELECT mode,quantity,retired_quantity,credits,native_minor FROM core_commerce_external.refunds WHERE order_id=$1 AND state NOT IN ('rejected','cancelled')")
            .bind(&key).fetch_all(&mut *tx).await?;
        let mut allocated=Allocated::default();
        for row in rows {
            let credits:i64=row.try_get("credits")?;let minor:i64=row.try_get("native_minor")?;let retired:i64=row.try_get("retired_quantity")?;
            allocated.credits=allocated.credits.checked_add(credits).ok_or(Error::Conflict)?;
            allocated.minor=allocated.minor.checked_add(minor).ok_or(Error::Conflict)?;
            allocated.retired=allocated.retired.checked_add(retired).ok_or(Error::Conflict)?;
            if row.try_get::<String,_>("mode")?=="quantity" {
                allocated.quantity=allocated.quantity.checked_add(row.try_get("quantity")?).ok_or(Error::Conflict)?;
                allocated.quantity_minor=allocated.quantity_minor.checked_add(minor).ok_or(Error::Conflict)?;
            }
        }
        let (credits,minor,retired)=plan_refund(request,basis.charged_minor,&allocated)?;
        let state=if request.order.method=="external:epay" {"manual"} else {"approved"};
        sqlx::query("INSERT INTO core_commerce_external.refunds(id,order_id,basis_id,command_sha256,command,authorized_by,mode,quantity,retired_quantity,credits,native_minor,state) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)")
            .bind(&refund).bind(&key).bind(&basis.id).bind(hash).bind(Json(request)).bind(actor.user_id).bind(&request.mode)
            .bind(request.quantity).bind(retired).bind(credits).bind(minor).bind(state).execute(&mut *tx).await?;
        let receipt=saved_refund(&mut tx,&refund).await?;tx.commit().await?;Ok(receipt)
    }

    /// Explicit current seller action only. CAS is committed BEFORE handing
    /// an executor permission to submit. A lost ACK returns no second permit.
    /// Query, timeout, lease expiry and restart never lower submit_count.
    pub(crate) async fn claim_dispatch(&self,secret:&str,request:&ExternalRefund)->Result<Option<DispatchPermit>> {
        let key=order_id(&request.order);
        let refund=id(&serde_json::json!(["store-external-refund-v1",key,request.request_key]));
        let mut tx=self.commerce.begin().await?;lock_order(&mut tx,&key).await?;
        let actor=crate::identity::authorize_in_transaction(&mut tx,secret).await?;
        let seller=native(&mut tx,request.order.seller,true).await?;
        authority(&mut tx,&actor,request.order.seller,seller,false).await?;
        let principal:Option<String>=sqlx::query_scalar("SELECT service_principal FROM core_commerce_external.orders WHERE id=$1 AND command_sha256=$2 AND seller_account_id=$3")
            .bind(&key).bind(digest(&request.order)?).bind(seller).fetch_optional(&mut *tx).await?;
        if principal.as_deref()!=Some(self.principal.as_str()) {return Err(Error::Forbidden)}
        // Platform submission must wait for the next slice binding the actual
        // original seller-income journal and its bounded reversible amount.
        // Keeping this gate before CAS cannot dispatch or fake wallet effects.
        if request.order.method!="external:waffo_pancake" {return Err(Error::Invalid)}
        let fence=id(&serde_json::json!(["store-provider-submit-v1",refund,hex(&digest(request)?)]));
        let claimed:Option<String>=sqlx::query_scalar("UPDATE core_commerce_external.refunds SET submit_count=1,dispatch_fence=$4,state='unknown' WHERE id=$1 AND order_id=$2 AND command_sha256=$3 AND submit_count=0 AND state='approved' RETURNING dispatch_fence")
            .bind(&refund).bind(&key).bind(digest(request)?).bind(&fence).fetch_optional(&mut *tx).await?;
        tx.commit().await?;Ok(claimed.map(|fence|DispatchPermit{refund_id:refund,fence}))
    }

    /// Service-only exact original-operation lookup. This never creates an
    /// order/refund, submits HTTP, admits evidence or changes a money counter.
    pub async fn reconcile_refund(&self,request:&ExternalRefund)->Result<Option<RefundReceipt>> {
        let key=order_id(&request.order);
        let refund=id(&serde_json::json!(["store-external-refund-v1",key,request.request_key]));
        let mut tx=self.commerce.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY").execute(&mut *tx).await?;
        let row=sqlx::query("SELECT o.service_principal,o.command_sha256 AS original_hash,r.command_sha256 FROM core_commerce_external.orders o JOIN core_commerce_external.refunds r ON r.order_id=o.id WHERE o.id=$1 AND r.id=$2")
            .bind(&key).bind(&refund).fetch_optional(&mut *tx).await?;
        let Some(row)=row else {tx.commit().await?;return Ok(None)};
        if row.try_get::<String,_>("service_principal")?!=self.principal {return Err(Error::Forbidden)}
        if row.try_get::<Vec<u8>,_>("original_hash")?!=digest(&request.order)? || row.try_get::<Vec<u8>,_>("command_sha256")?!=digest(request)? {return Err(Error::Conflict)}
        let receipt=saved_refund(&mut tx,&refund).await?;tx.commit().await?;Ok(Some(receipt))
    }
}

impl ExternalCommerce {
    pub fn new(pool:PgPool,payments:PaymentsStore,principal:&str)->Result<Self> {
        let commerce=CommerceStore::from_pool(pool).with_service_principal(principal)?;
        Ok(Self{commerce,payments,principal:principal.into()})
    }

    /// Current buyer authorization freezes the exact original quote. The
    /// merchant/channel association must already exist in Core owner material.
    /// This method creates neither a payment nor a wallet journal.
    pub async fn prepare_order(&self,secret:&str,order:&ExternalOrder)->Result<OrderReceipt> {
        validate_order(order)?;
        let scope=self.payments.store_channel_scope(order.channel_id,&order.method).map_err(payment_error)?;
        if scope.currency!=order.currency || order.quoted_minor>scope.maximum_minor {return Err(Error::Invalid)}
        if order.method.starts_with("platform:") && (scope.credits_per_minor<=0
            || i128::from(order.principal_credits)>i128::from(order.quoted_minor)*i128::from(scope.credits_per_minor)) {
            return Err(Error::Invalid)
        }
        let mut tx=self.commerce.begin().await?;
        let actor=crate::identity::authorize_in_transaction(&mut tx,secret).await?;
        // External checkout is not permission to debit an API key's wallet or
        // bypass its funding budget. That separate grant is not implemented here.
        if actor.credential_kind==CredentialKind::ApiKey {return Err(Error::Forbidden)}
        let buyer=native(&mut tx,order.buyer,true).await?;
        let seller=native(&mut tx,order.seller,true).await?;
        authority(&mut tx,&actor,order.buyer,buyer,true).await?;
        let key=order_id(order);lock_order(&mut tx,&key).await?;
        let hash=digest(order)?;
        if let Some(row)=sqlx::query("SELECT command_sha256,service_principal,provider_order_reference,provider_buyer_reference FROM core_commerce_external.orders WHERE id=$1")
            .bind(&key).fetch_optional(&mut *tx).await? {
            if row.try_get::<Vec<u8>,_>("command_sha256")?!=hash || row.try_get::<String,_>("service_principal")?!=self.principal {return Err(Error::Conflict)}
            let receipt=OrderReceipt{id:key,command_hash:hex(&hash),provider_order_reference:row.try_get("provider_order_reference")?,provider_buyer_reference:row.try_get("provider_buyer_reference")?};
            tx.commit().await?;return Ok(receipt)
        }
        let allowed:bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM core_commerce_external.merchant_channels WHERE seller_account_id=$1 AND channel_id=$2 AND method=$3 AND active AND scope_sha256=$4)")
            .bind(seller).bind(order.channel_id).bind(&order.method).bind(digest(&scope)?).fetch_one(&mut *tx).await?;
        if !allowed {return Err(Error::Forbidden)}
        let reference=format!("store_{key}");let buyer_ref=format!("store_buyer_{buyer}");
        sqlx::query("INSERT INTO core_commerce_external.orders(id,shop_id,order_id,service_principal,command_sha256,command,channel_scope,buyer_account_id,seller_account_id,actor_user_id,provider_order_reference,provider_buyer_reference,quantity,principal_credits,quoted_minor,currency) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16)")
            .bind(&key).bind(&order.shop_id).bind(&order.order_id).bind(&self.principal).bind(&hash).bind(Json(order)).bind(Json(scope))
            .bind(buyer).bind(seller).bind(actor.user_id).bind(&reference).bind(&buyer_ref).bind(order.quantity).bind(order.principal_credits).bind(order.quoted_minor).bind(&order.currency).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(OrderReceipt{id:key,command_hash:hex(&hash),provider_order_reference:reference,provider_buyer_reference:buyer_ref})
    }

    /// Core verifies the ORIGINAL signed payment, not a Go-asserted receipt.
    /// No Action::Credit, top-up proof, seller income or delivery is applied.
    pub async fn admit_payment_basis(&self,key:&str,payload:&[u8],signature:&[u8],now:i64)->Result<BasisReceipt> {
        if !hash_id(key) {return Err(Error::Invalid)}
        let mut tx=self.commerce.begin().await?;lock_order(&mut tx,key).await?;
        let row=sqlx::query("SELECT service_principal,channel_scope,provider_order_reference,provider_buyer_reference,quoted_minor FROM core_commerce_external.orders WHERE id=$1 FOR SHARE")
            .bind(key).fetch_optional(&mut *tx).await?.ok_or(Error::Missing)?;
        if row.try_get::<String,_>("service_principal")?!=self.principal {return Err(Error::Forbidden)}
        let prior=sqlx::query("SELECT signed_payload,signature,facts_sha256 FROM core_commerce_external.payment_bases WHERE order_id=$1").bind(key).fetch_optional(&mut *tx).await?;
        if let Some(prior)=&prior {
            if prior.try_get::<Vec<u8>,_>("signed_payload")?==payload && prior.try_get::<Vec<u8>,_>("signature")?==signature {
                let receipt=Self::basis(&mut tx,key).await?;tx.commit().await?;return Ok(receipt)
            }
        }
        let Json(scope):Json<StoreChannelScope>=row.try_get("channel_scope")?;
        let original:String=row.try_get("provider_order_reference")?;
        let buyer:String=row.try_get("provider_buyer_reference")?;
        let proof=self.payments.verify_store_charge(&scope,&original,&buyer,row.try_get("quoted_minor")?,payload,signature,now).map_err(payment_error)?;
        if let Some(prior)=prior {
            if prior.try_get::<Vec<u8>,_>("facts_sha256")?!=proof.facts_sha256() {return Err(Error::Conflict)}
            let receipt=Self::basis(&mut tx,key).await?;tx.commit().await?;return Ok(receipt)
        }
        let id=id(&serde_json::json!(["store-payment-basis-v1",key,hex(proof.facts_sha256())]));
        sqlx::query("INSERT INTO core_commerce_external.payment_bases(order_id,id,provider,merchant,environment,provider_order_reference,receipt_reference,payment_reference,charged_minor,currency,facts_sha256,payload_sha256,signed_payload,signature) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)")
            .bind(key).bind(id).bind(&scope.provider).bind(&scope.merchant).bind(&scope.environment).bind(original).bind(proof.receipt()).bind(proof.payment())
            .bind(proof.charged_minor()).bind(&scope.currency).bind(proof.facts_sha256()).bind(Sha256::digest(payload).to_vec()).bind(payload).bind(signature).execute(&mut *tx).await?;
        let receipt=Self::basis(&mut tx,key).await?;tx.commit().await?;Ok(receipt)
    }

    async fn basis(tx:&mut Transaction<'_,Postgres>,key:&str)->Result<BasisReceipt> {
        sqlx::query_as("SELECT id,receipt_reference,payment_reference,charged_minor,currency,facts_sha256 FROM core_commerce_external.payment_bases WHERE order_id=$1")
            .bind(key).fetch_optional(&mut **tx).await?.ok_or(Error::Missing)
    }
}

#[cfg(test)]
mod tests {
    // Local synthetic provider signatures and isolated real PG only. These
    // prove no live PSP execution and do not claim Core63 startup acceptance.
    use super::*;
    use crate::{accounts::AccountKind,core_schema,identity::IdentityStore,payments::{Channel,ProviderVerification}};
    use base64::{Engine,engine::general_purpose::STANDARD};
    use rand::{SeedableRng,rngs::StdRng};
    use rsa::{RsaPrivateKey,Pkcs1v15Sign,pkcs8::EncodePublicKey};
    use std::sync::OnceLock;
    const NOW:i64=1_900_000_000;
    fn private_key()->&'static RsaPrivateKey {
        static KEY:OnceLock<RsaPrivateKey>=OnceLock::new();
        KEY.get_or_init(||RsaPrivateKey::new(&mut StdRng::seed_from_u64(02026011002),2048).unwrap())
    }
    fn signed(order:&OrderReceipt,charged:&str,event_id:&str)->(Vec<u8>,Vec<u8>) {
        let payload=serde_json::to_vec(&serde_json::json!({"id":event_id,"eventType":"order.completed","mode":"test","storeId":"STO_fixture",
            "data":{"orderMerchantExternalId":order.provider_order_reference,"merchantProvidedBuyerIdentity":order.provider_buyer_reference,
            "currency":"USD","amount":"1.00","chargedAmount":charged,"paymentStatus":"succeeded","orderStatus":"completed",
            "orderId":"ORD_fixture","paymentId":"PAY_fixture"}})).unwrap();
        let timestamp=(NOW*1000).to_string();let mut bytes=timestamp.as_bytes().to_vec();bytes.push(b'.');bytes.extend(&payload);
        let signature=private_key().sign(Pkcs1v15Sign::new::<Sha256>(),&Sha256::digest(bytes)).unwrap();
        (payload,format!("t={timestamp},v1={}",STANDARD.encode(signature)).into_bytes())
    }
    async fn fixture(pool:PgPool)->(ExternalCommerce,ExternalOrder,String,String) {
        core_schema::init(&pool).await.unwrap();
        sqlx::raw_sql(SCHEMA).execute(&pool).await.unwrap();
        let identity=IdentityStore::from_pool(pool.clone());
        let buyer=identity.bootstrap_user(7,1).await.unwrap().secret;
        let seller=identity.bootstrap_user(9,1).await.unwrap().secret;
        let channel=Channel{id:71,provider:"waffo_pancake".into(),merchant:"merchant-fixture".into(),environment:"test".into(),currency:"USD".into(),
            webhook_secrets:vec![private_key().to_public_key().to_public_key_pem(Default::default()).unwrap()],
            credits_per_minor:5000,max_amount_minor:10000,verification:ProviderVerification{store_id:"STO_fixture".into(),product_id:"PROD_fixture".into(),..Default::default()}};
        let payments=PaymentsStore::new(pool.clone(),vec![channel]).unwrap();
        let scope=payments.store_channel_scope(71,"external:waffo_pancake").unwrap();
        let seller_native=identity.authorize(&seller).await.unwrap().owner_account_id;
        sqlx::query("INSERT INTO core_commerce_external.merchant_channels(seller_account_id,channel_id,method,scope_sha256) VALUES($1,71,'external:waffo_pancake',$2)")
            .bind(seller_native).bind(digest(&scope).unwrap()).execute(&pool).await.unwrap();
        let store=ExternalCommerce::new(pool,payments,"store-external-fixture").unwrap();
        let order=ExternalOrder{shop_id:"a".repeat(64),order_id:"b".repeat(64),buyer:Account{kind:AccountKind::Personal,id:7},seller:Account{kind:AccountKind::Personal,id:9},
            quantity:2,unit_credits:250000,principal_credits:500000,quoted_minor:100,currency:"USD".into(),quote_version:"fixture-only-v1".into(),channel_id:71,method:"external:waffo_pancake".into()};
        (store,order,buyer,seller)
    }
    fn refund(order:&ExternalOrder,basis:&BasisReceipt,key:&str)->ExternalRefund {
        ExternalRefund{order:order.clone(),basis_id:basis.id.clone(),request_key:key.into(),mode:"amount".into(),quantity:0,amount_credits:0,amount_minor:70,items_digest:"e".repeat(64)}
    }

    #[sqlx::test(migrations=false)]
    async fn goods_native_basis_uses_signed_charge_not_quote_and_never_credits_a_wallet(pool:PgPool) {
        let (store,order,buyer,_)=fixture(pool.clone()).await;
        let receipt=store.prepare_order(&buyer,&order).await.unwrap();
        let (payload,signature)=signed(&receipt,"1.20","delivery-one");
        let basis=store.admit_payment_basis(&receipt.id,&payload,&signature,NOW).await.unwrap();
        assert_eq!(basis.charged_minor,120);assert_eq!(basis.payment_reference,"PAY_fixture");assert_eq!(basis.receipt_reference,"ORD_fixture");
        let (retry,retry_sig)=signed(&receipt,"1.20","delivery-two");
        assert_eq!(store.admit_payment_basis(&receipt.id,&retry,&retry_sig,NOW).await.unwrap(),basis);
        assert_eq!(store.admit_payment_basis(&receipt.id,&payload,b"unverified",NOW).await,Err(Error::Forbidden));
        let journals:i64=sqlx::query_scalar("SELECT count(*) FROM core_billing.ledger_journals").fetch_one(&pool).await.unwrap();assert_eq!(journals,0);
        let (changed,changed_sig)=signed(&receipt,"1.21","delivery-three");
        assert_eq!(store.admit_payment_basis(&receipt.id,&changed,&changed_sig,NOW).await,Err(Error::Conflict));
    }

    #[sqlx::test(migrations=false)]
    async fn goods_refund_concurrent_caps_and_single_dispatch_survive_lost_ack(pool:PgPool) {
        let (store,order,buyer,seller)=fixture(pool.clone()).await;
        let receipt=store.prepare_order(&buyer,&order).await.unwrap();let (payload,signature)=signed(&receipt,"1.00","delivery-one");
        let basis=store.admit_payment_basis(&receipt.id,&payload,&signature,NOW).await.unwrap();
        let a=refund(&order,&basis,"first");let b=refund(&order,&basis,"competing");
        let (ar,br)=tokio::join!(store.prepare_refund(&seller,&a),store.prepare_refund(&seller,&b));
        assert_eq!(usize::from(ar.is_ok())+usize::from(br.is_ok()),1);
        let accepted=if ar.is_ok(){&a}else{&b};let accepted_receipt=if let Ok(r)=ar{r}else{br.unwrap()};
        assert_eq!(accepted_receipt.state,"approved");assert_eq!(accepted_receipt.submit_count,0);
        assert!(matches!(store.claim_dispatch(&buyer,accepted).await,Err(Error::Forbidden)));
        let (ca,cb)=tokio::join!(store.claim_dispatch(&seller,accepted),store.claim_dispatch(&seller,accepted));
        let ca=ca.unwrap();let cb=cb.unwrap();assert_eq!(usize::from(ca.is_some())+usize::from(cb.is_some()),1);
        let saved=store.reconcile_refund(accepted).await.unwrap().unwrap();assert_eq!(saved.state,"unknown");assert_eq!(saved.submit_count,1);
        assert!(store.claim_dispatch(&seller,accepted).await.unwrap().is_none());
        assert!(sqlx::query("UPDATE core_commerce_external.refunds SET submit_count=0,dispatch_fence=NULL,state='approved' WHERE id=$1").bind(saved.id).execute(&pool).await.is_err());
        let journals:i64=sqlx::query_scalar("SELECT count(*) FROM core_billing.ledger_journals").fetch_one(&pool).await.unwrap();assert_eq!(journals,0);
    }

    #[test]
    fn native_and_credit_caps_exhaust_together_and_never_use_fx() {
        let order=ExternalOrder{shop_id:"a".repeat(64),order_id:"b".repeat(64),buyer:Account{kind:AccountKind::Personal,id:7},seller:Account{kind:AccountKind::Personal,id:9},quantity:3,unit_credits:100,principal_credits:300,quoted_minor:100,currency:"USD".into(),quote_version:"v1".into(),channel_id:1,method:"external:waffo_pancake".into()};
        let basis=BasisReceipt{id:"c".repeat(64),receipt_reference:"ORD_fixture".into(),payment_reference:"PAY_fixture".into(),charged_minor:101,currency:"USD".into(),facts_sha256:vec![0;32]};
        let mut r=refund(&order,&basis,"quantity");r.mode="quantity".into();r.quantity=1;r.amount_minor=0;
        assert_eq!(plan_refund(&r,101,&Allocated::default()).unwrap(),(100,33,1));
        let a=Allocated{credits:200,minor:67,quantity:2,quantity_minor:67,retired:2};
        assert_eq!(plan_refund(&r,101,&a).unwrap(),(100,34,1));
    }
}
