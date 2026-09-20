mod alipay;
mod model;
mod secret;
mod service_impl;
#[cfg(test)]
mod tests;

pub use model::BillingError;
pub use service_impl::{MeterCommand, MeterResult};

/// 测试辅助：暴露支付宝异步通知的待签名串构造规则。
#[cfg(test)]
pub(crate) fn alipay_signing_content_for_test(fields: &[(String, String)]) -> String {
    alipay::notification_content(
        fields
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str())),
    )
}
