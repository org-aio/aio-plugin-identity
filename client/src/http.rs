use aio_plugin_identity_model::{
    IdentityErrorResponse, IdentityResponse, LoginRequest, PasswordRequest, RegisterRequest,
    SessionView,
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
