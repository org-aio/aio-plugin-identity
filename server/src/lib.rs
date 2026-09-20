mod password;
mod routes;
mod service;

pub use service::{BillingError, MeterCommand, MeterResult};
pub use service::{IdentityService, SessionContext};

use anyhow::{Context as _, Result};
use axum::Router;
use dill::CatalogBuilder;

pub fn register(builder: &mut CatalogBuilder) -> Result<()> {
    builder.add_value(IdentityService::from_env()?);
    Ok(())
}

pub fn service(catalog: &dill::Catalog) -> Result<std::sync::Arc<IdentityService>> {
    catalog
        .get_one::<IdentityService>()
        .context("身份服务未注册")
}

pub fn router(catalog: &dill::Catalog) -> Result<Router> {
    Ok(routes::router(service(catalog)?))
}
