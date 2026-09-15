#[derive(Debug)]
pub(crate) enum RegistrationError {
    Invalid(String),
    AccountTaken,
    Busy,
}

impl std::fmt::Display for RegistrationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(message) => f.write_str(message),
            Self::AccountTaken => f.write_str("账号已被使用"),
            Self::Busy => f.write_str("注册请求较多，请稍后重试"),
        }
    }
}

impl std::error::Error for RegistrationError {}
