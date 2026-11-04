use super::*;
use subtle::ConstantTimeEq;
use tonic::{Request, Response, Status, metadata::MetadataMap};
#[derive(Clone, Copy, Default)]
pub struct Permissions {
    pub pay: bool,
    pub refund: bool,
    pub read: bool,
    pub reconcile: bool,
}
#[derive(Clone)]
pub struct DirectoryAdsRpc {
    store: Store,
    digest: [u8; 32],
    principal: String,
    permissions: Permissions,
}
fn single<'a>(m: &'a MetadataMap, k: &'static str) -> std::result::Result<&'a str, Status> {
    let mut values = m.get_all(k).iter();
    let value = values
        .next()
        .ok_or_else(|| Status::unauthenticated("missing advertisement credential"))?;
    if values.next().is_some() {
        return Err(Status::unauthenticated(
            "duplicate advertisement credential",
        ));
    }
    value
        .to_str()
        .map_err(|_| Status::unauthenticated("invalid advertisement credential"))
}
fn status(e: Error) -> Status {
    match e {
        Error::Invalid => Status::invalid_argument("invalid advertisement command"),
        Error::Unauthorized => Status::unauthenticated("invalid current user credential"),
        Error::Forbidden => Status::permission_denied("advertisement authority denied"),
        Error::Conflict => Status::already_exists("advertisement command conflict"),
        Error::Expired => Status::failed_precondition("AD_QUOTE_EXPIRED"),
        Error::ConfirmationRequired => Status::failed_precondition("AD_CONFIRMATION_REQUIRED"),
        _ => Status::unavailable("advertisement outcome may be unknown; retry same key"),
    }
}
impl DirectoryAdsRpc {
    pub fn new(
        store: Store,
        token: &[u8],
        principal: String,
        permissions: Permissions,
    ) -> Result<Self> {
        if !(32..=256).contains(&token.len())
            || !token.iter().all(u8::is_ascii_graphic)
            || !key(&principal)
        {
            return Err(Error::Invalid);
        }
        Ok(Self {
            store,
            digest: Sha256::digest(token).into(),
            principal,
            permissions,
        })
    }
    pub fn features(&self) -> Vec<String> {
        [
            (self.permissions.pay, "directory_ads.pay.v1"),
            (self.permissions.refund, "directory_ads.refund.v1"),
            (self.permissions.read, "directory_ads.read.v1"),
            (self.permissions.reconcile, "directory_ads.reconcile.v1"),
        ]
        .into_iter()
        .filter(|(ok, _)| *ok)
        .map(|(_, s)| s.into())
        .collect()
    }
    fn check(&self, m: &MetadataMap, allowed: bool) -> std::result::Result<(), Status> {
        let token = single(m, "authorization")?
            .strip_prefix("Bearer ")
            .ok_or_else(|| Status::unauthenticated("invalid advertisement service credential"))?;
        let digest: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        if !bool::from(self.digest.ct_eq(&digest)) {
            return Err(Status::unauthenticated(
                "invalid advertisement service credential",
            ));
        }
        if single(m, "x-lmm-protocol")? != "1" {
            return Err(Status::failed_precondition("unsupported protocol"));
        }
        if !allowed {
            return Err(Status::permission_denied(
                "advertisement service permission denied",
            ));
        }
        Ok(())
    }
}
#[tonic::async_trait]
impl pb::core_directory_ads_server::CoreDirectoryAds for DirectoryAdsRpc {
    async fn prepare_directory_ad(
        &self,
        r: Request<pb::PrepareDirectoryAdRequest>,
    ) -> std::result::Result<Response<pb::DirectoryAdQuote>, Status> {
        self.check(r.metadata(), self.permissions.pay)?;
        self.store
            .prepare(
                &self.principal,
                single(r.metadata(), "x-lmm-user-credential")?,
                r.get_ref(),
            )
            .await
            .map(Response::new)
            .map_err(status)
    }
    async fn pay_directory_ad(
        &self,
        r: Request<pb::PayDirectoryAdRequest>,
    ) -> std::result::Result<Response<pb::DirectoryAdReceipt>, Status> {
        self.check(r.metadata(), self.permissions.pay)?;
        self.store
            .pay(
                &self.principal,
                single(r.metadata(), "x-lmm-user-credential")?,
                r.get_ref(),
            )
            .await
            .map(Response::new)
            .map_err(status)
    }
    async fn query_directory_ad_payment(
        &self,
        r: Request<pb::PayDirectoryAdRequest>,
    ) -> std::result::Result<Response<pb::DirectoryAdResult>, Status> {
        self.check(r.metadata(), self.permissions.read)?;
        self.store
            .query_payment(
                &self.principal,
                Some(single(r.metadata(), "x-lmm-user-credential")?),
                r.get_ref(),
            )
            .await
            .map(|receipt| Response::new(pb::DirectoryAdResult { receipt }))
            .map_err(status)
    }
    async fn reconcile_directory_ad_payment(
        &self,
        r: Request<pb::PayDirectoryAdRequest>,
    ) -> std::result::Result<Response<pb::DirectoryAdResult>, Status> {
        self.check(r.metadata(), self.permissions.reconcile)?;
        self.store
            .query_payment(&self.principal, None, r.get_ref())
            .await
            .map(|receipt| Response::new(pb::DirectoryAdResult { receipt }))
            .map_err(status)
    }
    async fn refund_directory_ad(
        &self,
        r: Request<pb::RefundDirectoryAdRequest>,
    ) -> std::result::Result<Response<pb::DirectoryAdReceipt>, Status> {
        self.check(r.metadata(), self.permissions.refund)?;
        self.store
            .refund(
                &self.principal,
                single(r.metadata(), "x-lmm-user-credential")?,
                r.get_ref(),
            )
            .await
            .map(Response::new)
            .map_err(status)
    }
    async fn query_directory_ad_refund(
        &self,
        r: Request<pb::RefundDirectoryAdRequest>,
    ) -> std::result::Result<Response<pb::DirectoryAdResult>, Status> {
        self.check(r.metadata(), self.permissions.read)?;
        self.store
            .query_refund(
                &self.principal,
                Some(single(r.metadata(), "x-lmm-user-credential")?),
                r.get_ref(),
            )
            .await
            .map(|receipt| Response::new(pb::DirectoryAdResult { receipt }))
            .map_err(status)
    }
    async fn reconcile_directory_ad_refund(
        &self,
        r: Request<pb::RefundDirectoryAdRequest>,
    ) -> std::result::Result<Response<pb::DirectoryAdResult>, Status> {
        self.check(r.metadata(), self.permissions.reconcile)?;
        self.store
            .query_refund(&self.principal, None, r.get_ref())
            .await
            .map(|receipt| Response::new(pb::DirectoryAdResult { receipt }))
            .map_err(status)
    }
}
