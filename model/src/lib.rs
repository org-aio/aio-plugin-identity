use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LoginRequest {
    pub account: String,
    pub password: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisterRequest {
    pub account: String,
    pub password: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SessionView {
    pub user_id: String,
    pub account: String,
    pub display_name: String,
    pub tenant_id: String,
    pub tenant_label: String,
    pub permissions: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct IdentityResponse<T> {
    pub data: T,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct IdentityErrorResponse {
    pub error: String,
}

/// 钱包视图：余额以微单位（1e-6 货币单位）存储，避免浮点误差。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WalletView {
    pub currency: String,
    pub balance_micros: i64,
    pub subscription: Option<SubscriptionView>,
    pub grants: Vec<UsageGrantView>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub id: String,
    pub kind: String,
    pub amount_micros: i64,
    pub balance_after_micros: i64,
    pub description: String,
    pub created_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LedgerPage {
    pub entries: Vec<LedgerEntry>,
    /// 下一页游标；为 `None` 表示已到末尾。
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PlanView {
    pub code: String,
    pub title: String,
    pub price_micros: i64,
    pub currency: String,
    pub interval: String,
    pub included_usage: Vec<PlanUsage>,
    pub features: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PlanUsage {
    pub resource: String,
    pub quantity: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SubscriptionView {
    pub plan: PlanView,
    pub status: String,
    pub period_start: String,
    pub period_end: String,
    pub auto_renew: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct UsageGrantView {
    pub resource: String,
    pub quantity: i64,
    pub consumed: i64,
    pub period_end: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RechargeRequest {
    pub amount_micros: i64,
    pub provider: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RechargeOrderView {
    pub id: String,
    pub amount_micros: i64,
    pub currency: String,
    pub provider: String,
    pub status: String,
    pub created_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubscribeRequest {
    pub plan_code: String,
}
