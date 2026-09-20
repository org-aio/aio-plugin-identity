use aio_plugin_identity_model::{
    IdentityErrorResponse, IdentityResponse, LedgerPage, LoginRequest, PasswordRequest,
    RechargeOrderView, RechargeRequest, RegisterRequest, SessionView, SubscribeRequest, WalletView,
};
use gloo_net::http::Request;

pub async fn load_session() -> Result<Option<SessionView>, String> {
    let response = Request::get("/api/auth/session")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.ok() {
        return Err(response.text().await.unwrap_or_default());
    }
    response
        .json::<IdentityResponse<Option<SessionView>>>()
        .await
        .map(|r| r.data)
        .map_err(|e| e.to_string())
}

pub(super) async fn login(request: LoginRequest) -> Result<(), String> {
    send(
        "/api/auth/login",
        serde_json::to_string(&request).map_err(|e| e.to_string())?,
    )
    .await?;
    refresh_session().await
}

pub(super) async fn register_account(request: RegisterRequest) -> Result<(), String> {
    send(
        "/api/auth/register",
        serde_json::to_string(&request).map_err(|e| e.to_string())?,
    )
    .await?;
    refresh_session().await
}

async fn refresh_session() -> Result<(), String> {
    dioxus::document::eval(
        "window.dispatchEvent(new Event('aio:catalog-invalidated')); return true;",
    )
    .await
    .map(|_| ())
    .map_err(|e| e.to_string())
}

pub(super) async fn change_password(request: PasswordRequest) -> Result<(), String> {
    send(
        "/api/auth/password",
        serde_json::to_string(&request).map_err(|e| e.to_string())?,
    )
    .await
}

pub(crate) async fn load_wallet() -> Result<WalletView, String> {
    get("/api/billing/wallet").await
}

pub(crate) async fn load_ledger(cursor: Option<String>) -> Result<LedgerPage, String> {
    let path = match cursor {
        Some(cursor) => format!("/api/billing/ledger?cursor={}", encode(&cursor)),
        None => "/api/billing/ledger".to_owned(),
    };
    get(&path).await
}

pub(crate) async fn create_recharge_order(
    request: RechargeRequest,
) -> Result<RechargeOrderView, String> {
    post_json("/api/billing/orders", &request).await
}

pub(crate) async fn confirm_recharge_order(id: &str) -> Result<WalletView, String> {
    let response = Request::post(&format!("/api/billing/orders/{id}/confirm"))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    decode(response).await
}

pub(crate) async fn subscribe(request: SubscribeRequest) -> Result<WalletView, String> {
    post_json("/api/billing/subscription", &request).await
}

async fn get<T: serde::de::DeserializeOwned>(path: &str) -> Result<T, String> {
    let response = Request::get(path).send().await.map_err(|e| e.to_string())?;
    decode(response).await
}

async fn post_json<T: serde::de::DeserializeOwned, B: serde::Serialize>(
    path: &str,
    body: &B,
) -> Result<T, String> {
    let response = Request::post(path)
        .header("content-type", "application/json")
        .body(serde_json::to_string(body).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?
        .send()
        .await
        .map_err(|e| e.to_string())?;
    decode(response).await
}

async fn decode<T: serde::de::DeserializeOwned>(
    response: gloo_net::http::Response,
) -> Result<T, String> {
    if !response.ok() {
        let body = response.text().await.unwrap_or_default();
        return Err(serde_json::from_str::<IdentityErrorResponse>(&body)
            .map(|r| r.error)
            .unwrap_or(body));
    }
    response
        .json::<IdentityResponse<T>>()
        .await
        .map(|r| r.data)
        .map_err(|e| e.to_string())
}

/// 最小百分号编码，只用于账本游标这类受控 ID。
fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                char::from(byte).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

async fn send(path: &str, body: String) -> Result<(), String> {
    let response = Request::post(path)
        .header("content-type", "application/json")
        .body(body)
        .map_err(|e| e.to_string())?
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if response.ok() {
        return Ok(());
    }
    let body = response.text().await.unwrap_or_default();
    Err(serde_json::from_str::<IdentityErrorResponse>(&body)
        .map(|r| r.error)
        .unwrap_or(body))
}
