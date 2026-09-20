mod model;
mod service_impl;
#[cfg(test)]
mod tests;

pub use model::BillingError;
pub use service_impl::{MeterCommand, MeterResult};
