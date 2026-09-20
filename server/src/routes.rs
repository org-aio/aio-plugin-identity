use std::sync::Arc;

use aio_plugin_identity_model::{
    IdentityErrorResponse, IdentityResponse, LedgerPage, LoginRequest, PasswordRequest,
    RechargeOrderView, RechargeRequest, RegisterRequest, SessionView, SubscribeRequest, WalletView,
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;

use crate::{BillingError, IdentityService, SessionContext};

pub fn router(service: Arc<IdentityService>) -> Router {
    Router::new()
        .route("/api/plugins/identity/health", get(health))
        .route("/api/auth/login", post(login))
        .route(
            "/api/auth/register",
            post(register_account).layer(DefaultBodyLimit::max(16 * 1024)),
        )
        .route("/api/auth/session", get(session))
        .route("/api/auth/logout", post(logout))
        .route("/api/auth/password", post(change_password))
        .route("/api/billing/wallet", get(wallet))
        .route("/api/billing/ledger", get(ledger))
        .route("/api/billing/orders", post(create_recharge_order))
        .route(
            "/api/billing/orders/{id}/confirm",
            post(confirm_recharge_order),
        )
        .route("/api/billing/subscription", post(subscribe))
        .with_state(service)
}

async fn health() -> &'static str {
    "ok"
}

async fn register_account(
    State(service): State<Arc<IdentityService>>,
    Json(request): Json<RegisterRequest>,
) -> Result<Response, IdentityHttpError> {
    use crate::service::RegistrationError;
    let result = service.register_account(&request).await.map_err(|error| {
        let status = match error.downcast_ref::<RegistrationError>() {
            Some(RegistrationError::Invalid(_)) => StatusCode::BAD_REQUEST,
            Some(RegistrationError::AccountTaken) => StatusCode::CONFLICT,
            Some(RegistrationError::Busy) => StatusCode::SERVICE_UNAVAILABLE,
            None => StatusCode::INTERNAL_SERVER_ERROR,
        };
        IdentityHttpError {
            status,
            message: if status == StatusCode::INTERNAL_SERVER_ERROR {
                "注册失败，请稍后重试".into()
            } else {
                error.to_string()
            },
        }
    })?;
    Ok((
        StatusCode::CREATED,
        [
            (header::SET_COOKIE, result.cookie),
            (header::CACHE_CONTROL, "no-store".into()),
        ],
        Json(IdentityResponse {
            data: result.session.view(),
        }),
    )
        .into_response())
}

async fn login(
    State(service): State<Arc<IdentityService>>,
    Json(request): Json<LoginRequest>,
) -> Result<Response, IdentityHttpError> {
    let result = service
        .login(&request)
        .await?
        .ok_or_else(|| IdentityHttpError::unauthorized("账号或密码错误"))?;
    Ok((
        [(header::SET_COOKIE, result.cookie)],
        Json(IdentityResponse {
            data: result.session.view(),
        }),
    )
        .into_response())
}

async fn session(
    State(service): State<Arc<IdentityService>>,
    headers: HeaderMap,
) -> Result<Json<IdentityResponse<Option<SessionView>>>, IdentityHttpError> {
    let session = service.authenticate(&headers).await?;
    Ok(Json(IdentityResponse {
        data: session.map(|session| session.view()),
    }))
}

async fn logout(
    State(service): State<Arc<IdentityService>>,
    headers: HeaderMap,
) -> Result<Response, IdentityHttpError> {
    service.logout(&headers).await?;
    Ok((
        [(header::SET_COOKIE, service.expired_cookie())],
        StatusCode::NO_CONTENT,
    )
        .into_response())
}

async fn change_password(
    State(service): State<Arc<IdentityService>>,
    headers: HeaderMap,
    Json(request): Json<PasswordRequest>,
) -> Result<StatusCode, IdentityHttpError> {
    let session = service
        .authenticate(&headers)
        .await?
        .ok_or_else(|| IdentityHttpError::unauthorized("会话无效或已过期"))?;
    if !service.change_password(&session, &request).await? {
        return Err(IdentityHttpError::unauthorized("当前密码错误"));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
struct LedgerQuery {
    cursor: Option<String>,
}

async fn wallet(
    State(service): State<Arc<IdentityService>>,
    headers: HeaderMap,
) -> Result<Json<IdentityResponse<WalletView>>, IdentityHttpError> {
    let session = authenticate(&service, &headers).await?;
    let wallet = service.wallet(&session).await.map_err(map_billing_error)?;
    Ok(Json(IdentityResponse { data: wallet }))
}

async fn ledger(
    State(service): State<Arc<IdentityService>>,
    headers: HeaderMap,
    Query(query): Query<LedgerQuery>,
) -> Result<Json<IdentityResponse<LedgerPage>>, IdentityHttpError> {
    let session = authenticate(&service, &headers).await?;
    let page = service
        .ledger(&session, query.cursor.as_deref())
        .await
        .map_err(map_billing_error)?;
    Ok(Json(IdentityResponse { data: page }))
}

async fn create_recharge_order(
    State(service): State<Arc<IdentityService>>,
    headers: HeaderMap,
    Json(request): Json<RechargeRequest>,
) -> Result<(StatusCode, Json<IdentityResponse<RechargeOrderView>>), IdentityHttpError> {
    let session = authenticate(&service, &headers).await?;
    require_billing_manage(&session)?;
    let order = service
        .create_recharge_order(&session, &request)
        .await
        .map_err(map_billing_error)?;
    Ok((StatusCode::CREATED, Json(IdentityResponse { data: order })))
}

async fn confirm_recharge_order(
    State(service): State<Arc<IdentityService>>,
    headers: HeaderMap,
    Path(order_id): Path<String>,
) -> Result<Json<IdentityResponse<WalletView>>, IdentityHttpError> {
    let session = authenticate(&service, &headers).await?;
    require_billing_manage(&session)?;
    let wallet = service
        .confirm_recharge_order(&session, &order_id)
        .await
        .map_err(map_billing_error)?;
    Ok(Json(IdentityResponse { data: wallet }))
}

async fn subscribe(
    State(service): State<Arc<IdentityService>>,
    headers: HeaderMap,
    Json(request): Json<SubscribeRequest>,
) -> Result<Json<IdentityResponse<WalletView>>, IdentityHttpError> {
    let session = authenticate(&service, &headers).await?;
    require_billing_manage(&session)?;
    let wallet = service
        .subscribe(&session, &request)
        .await
        .map_err(map_billing_error)?;
    Ok(Json(IdentityResponse { data: wallet }))
}

async fn authenticate(
    service: &IdentityService,
    headers: &HeaderMap,
) -> Result<SessionContext, IdentityHttpError> {
    service
        .authenticate(headers)
        .await?
        .ok_or_else(|| IdentityHttpError::unauthorized("会话无效或已过期"))
}

fn require_billing_manage(session: &SessionContext) -> Result<(), IdentityHttpError> {
    if session
        .permissions
        .iter()
        .any(|permission| permission == "billing:manage")
    {
        return Ok(());
    }
    Err(IdentityHttpError {
        status: StatusCode::FORBIDDEN,
        message: "当前角色没有计费管理权限".into(),
    })
}

fn map_billing_error(error: anyhow::Error) -> IdentityHttpError {
    let status = match error.downcast_ref::<BillingError>() {
        Some(BillingError::Invalid(_)) => StatusCode::BAD_REQUEST,
        Some(BillingError::NotFound(_)) => StatusCode::NOT_FOUND,
        Some(BillingError::InsufficientFunds) => StatusCode::PAYMENT_REQUIRED,
        None => StatusCode::INTERNAL_SERVER_ERROR,
    };
    IdentityHttpError {
        status,
        message: if status == StatusCode::INTERNAL_SERVER_ERROR {
            "计费服务暂时不可用".into()
        } else {
            error.to_string()
        },
    }
}

struct IdentityHttpError {
    status: StatusCode,
    message: String,
}

impl IdentityHttpError {
    fn unauthorized(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: message.into(),
        }
    }
}

impl<E> From<E> for IdentityHttpError
where
    E: Into<anyhow::Error>,
{
    fn from(value: E) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: format!("{:#}", value.into()),
        }
    }
}

impl IntoResponse for IdentityHttpError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(IdentityErrorResponse {
                error: self.message,
            }),
        )
            .into_response()
    }
}
