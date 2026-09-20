use thiserror::Error;

#[derive(Debug, Error)]
pub enum BillingError {
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    NotFound(String),
    #[error("余额不足")]
    InsufficientFunds,
}
