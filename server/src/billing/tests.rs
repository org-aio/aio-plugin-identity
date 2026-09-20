use super::super::{IdentityService, SCHEMA};
use super::{MeterCommand, MeterResult};
use anyhow::Result;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use serde_json::{Value, json};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use std::{str::FromStr, sync::Arc};
use tower::ServiceExt;

async fn request(
    router: axum::Router,
    method: &str,
    path: &str,
    cookie: Option<&str>,
    body: Option<Value>,
) -> Result<(StatusCode, Value)> {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    let body = match body {
        Some(value) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    let response = router.oneshot(builder.body(body)?).await?;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1_000_000).await?;
    Ok((
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    ))
}

async fn register(router: axum::Router, account: &str) -> Result<String> {
    let response = router
        .oneshot(
            Request::post("/api/auth/register")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"account": account, "password": "billing-test-password"}).to_string(),
                ))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::CREATED);
    Ok(response
        .headers()
        .get(header::SET_COOKIE)
        .expect("注册应返回会话 Cookie")
        .to_str()?
        .split(';')
        .next()
        .unwrap()
        .to_owned())
}

fn cookie_value(set_cookie: &str) -> &str {
    set_cookie.split(';').next().unwrap()
}

#[tokio::test]
#[ignore = "需要 AIO_IDENTITY_TEST_DATABASE_URL 指向本机独立 aio_identity_test 数据库"]
async fn wallet_recharge_subscription_and_metering_are_consistent() -> Result<()> {
    let url = std::env::var("AIO_IDENTITY_TEST_DATABASE_URL")?;
    let options = PgConnectOptions::from_str(&url)?;
    anyhow::ensure!(
        matches!(options.get_host(), "localhost" | "127.0.0.1" | "/tmp")
            && options.get_database() == Some("aio_identity_test"),
        "只允许独立本机测试库"
    );
    let root = PgPoolOptions::new().connect_with(options.clone()).await?;
    let schema = format!("billing_{}", uuid::Uuid::new_v4().simple());
    sqlx::raw_sql(&format!("CREATE SCHEMA {schema}"))
        .execute(&root)
        .await?;
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect_with(options.options([("search_path", schema.as_str())]))
        .await?;
    sqlx::raw_sql(SCHEMA).execute(&pool).await?;
    let service = Arc::new(IdentityService {
        pool: pool.clone(),
        secure_cookie: true,
        password_min_length: 12,
        registration_slots: tokio::sync::Semaphore::new(4),
    });
    service.seed_plans().await?;
    let router = crate::routes::router(service.clone());

    let cookie = register(router.clone(), "billing_user").await?;
    let cookie = cookie_value(&cookie).to_owned();

    // 新注册租户钱包为 0，且没有订阅。
    let (status, body) = request(
        router.clone(),
        "GET",
        "/api/billing/wallet",
        Some(&cookie),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["balance_micros"], 0);
    assert_eq!(body["data"]["currency"], "USD");
    assert!(body["data"]["subscription"].is_null());
    assert!(
        body["data"].get("tenant_id").is_none(),
        "钱包视图不暴露 tenant_id"
    );

    // 余额不足时按量计费被拒。
    let session = service
        .authenticate(&cookie_header(&cookie))
        .await?
        .expect("会话有效");
    let insufficient = service
        .meter(MeterCommand {
            tenant_id: session.tenant_id.clone(),
            user_id: session.user_id.clone(),
            source_id: "agent".into(),
            resource: "agent_tokens".into(),
            quantity: 1_000,
            unit_price_micros: 1_000,
            idempotency_key: "msg-1".into(),
        })
        .await
        .unwrap_err();
    assert!(insufficient.to_string().contains("余额不足"));

    // 创建并确认充值订单，余额入账且写账本。
    let (status, body) = request(
        router.clone(),
        "POST",
        "/api/billing/orders",
        Some(&cookie),
        Some(json!({"amount_micros": 60_000_000, "provider": "manual"})),
    )
    .await?;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["data"]["status"], "pending");
    let order_id = body["data"]["id"].as_str().unwrap().to_owned();
    let (status, body) = request(
        router.clone(),
        "POST",
        &format!("/api/billing/orders/{order_id}/confirm"),
        Some(&cookie),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["balance_micros"], 60_000_000);

    // 重复确认订单幂等，不重复入账。
    let (status, body) = request(
        router.clone(),
        "POST",
        &format!("/api/billing/orders/{order_id}/confirm"),
        Some(&cookie),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["balance_micros"], 60_000_000);

    // 账本至少有一条 recharge 记录。
    let (status, body) = request(
        router.clone(),
        "GET",
        "/api/billing/ledger",
        Some(&cookie),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let kinds = body["data"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["kind"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert!(kinds.contains(&"recharge".to_owned()));

    // 订阅套餐后获得周期额度。
    let (status, body) = request(
        router.clone(),
        "POST",
        "/api/billing/subscription",
        Some(&cookie),
        Some(json!({"plan_code": "agent_pro_60"})),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["subscription"]["status"], "active");
    assert_eq!(body["data"]["subscription"]["plan"]["code"], "agent_pro_60");
    assert_eq!(body["data"]["grants"][0]["resource"], "agent_tokens");
    assert_eq!(body["data"]["grants"][0]["quantity"], 20_000_000);

    // 先扣套餐额度：额度足够时不扣余额。
    let grant_hit = service
        .meter(MeterCommand {
            tenant_id: session.tenant_id.clone(),
            user_id: session.user_id.clone(),
            source_id: "agent".into(),
            resource: "agent_tokens".into(),
            quantity: 1_000,
            unit_price_micros: 1_000,
            idempotency_key: "msg-2".into(),
        })
        .await?;
    assert_eq!(
        grant_hit,
        MeterResult {
            amount_micros: 1_000_000,
            grant_consumed: 1_000,
            balance_charged_micros: 0,
            balance_after_micros: 60_000_000,
            duplicate: false,
        }
    );

    // 幂等键重复时不再扣费。
    let duplicate = service
        .meter(MeterCommand {
            tenant_id: session.tenant_id.clone(),
            user_id: session.user_id.clone(),
            source_id: "agent".into(),
            resource: "agent_tokens".into(),
            quantity: 1_000,
            unit_price_micros: 1_000,
            idempotency_key: "msg-2".into(),
        })
        .await?;
    assert!(duplicate.duplicate);
    assert_eq!(duplicate.balance_charged_micros, 0);

    // 额度耗尽后回落到余额：先把额度扣完，再上报一笔。
    let drain = service
        .meter(MeterCommand {
            tenant_id: session.tenant_id.clone(),
            user_id: session.user_id.clone(),
            source_id: "agent".into(),
            resource: "agent_tokens".into(),
            quantity: 20_000_000 - 1_000,
            unit_price_micros: 1_000,
            idempotency_key: "msg-3".into(),
        })
        .await?;
    assert_eq!(drain.balance_charged_micros, 0);
    let balance_hit = service
        .meter(MeterCommand {
            tenant_id: session.tenant_id.clone(),
            user_id: session.user_id.clone(),
            source_id: "agent".into(),
            resource: "agent_tokens".into(),
            quantity: 500,
            unit_price_micros: 1_000,
            idempotency_key: "msg-4".into(),
        })
        .await?;
    assert_eq!(balance_hit.grant_consumed, 0);
    assert_eq!(balance_hit.balance_charged_micros, 500_000);
    assert_eq!(balance_hit.balance_after_micros, 59_500_000);

    // 账本里出现 usage 扣费记录。
    let usage_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM billing_ledger WHERE kind = 'usage'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(usage_count, 1);

    // 未登录访问钱包返回 401。
    let (status, _) = request(router.clone(), "GET", "/api/billing/wallet", None, None).await?;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    Ok(())
}

/// 从 `aio_session=...` Cookie 片段构造请求头，供服务端鉴权复用。
fn cookie_header(cookie: &str) -> axum::http::HeaderMap {
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(header::COOKIE, cookie.parse().unwrap());
    headers
}

#[tokio::test]
#[ignore = "需要 AIO_IDENTITY_TEST_DATABASE_URL 指向本机独立 aio_identity_test 数据库"]
async fn alipay_channel_config_and_notify_are_verified() -> Result<()> {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use rsa::{
        Pkcs1v15Sign, RsaPrivateKey, RsaPublicKey,
        pkcs8::{EncodePrivateKey as _, EncodePublicKey as _},
    };
    use sha2::{Digest as _, Sha256};

    let url = std::env::var("AIO_IDENTITY_TEST_DATABASE_URL")?;
    let options = PgConnectOptions::from_str(&url)?;
    let root = PgPoolOptions::new().connect_with(options.clone()).await?;
    let schema = format!("alipay_{}", uuid::Uuid::new_v4().simple());
    sqlx::raw_sql(&format!("CREATE SCHEMA {schema}"))
        .execute(&root)
        .await?;
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect_with(options.options([("search_path", schema.as_str())]))
        .await?;
    sqlx::raw_sql(SCHEMA).execute(&pool).await?;
    let service = Arc::new(IdentityService {
        pool: pool.clone(),
        secure_cookie: true,
        password_min_length: 12,
        registration_slots: tokio::sync::Semaphore::new(4),
    });
    service.seed_plans().await?;
    let router = crate::routes::router(service.clone());
    let cookie = cookie_value(&register(router.clone(), "alipay_user").await?).to_owned();
    let session = service
        .authenticate(&cookie_header(&cookie))
        .await?
        .expect("会话有效");

    // 生成一对测试密钥，私钥加密入库，公钥用于验签。
    let mut rng = rand::thread_rng();
    let private = RsaPrivateKey::new(&mut rng, 2048)?;
    let public = RsaPublicKey::from(&private);
    let private_pem = private
        .to_pkcs8_pem(rsa::pkcs8::LineEnding::LF)?
        .to_string();
    let public_pem = public.to_public_key_pem(rsa::pkcs8::LineEnding::LF)?;

    // 未配置私钥时不能启用。
    let (status, body) = request(
        router.clone(),
        "PUT",
        "/api/billing/payment-channels/alipay",
        Some(&cookie),
        Some(json!({
            "enabled": true,
            "app_id": "2021000000000000",
            "gateway": "",
            "notify_url": "https://aio.addzero.site/api/billing/alipay/notify",
            "return_url": "https://aio.addzero.site/",
            "seller_id": "",
            "public_key": public_pem,
        })),
    )
    .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("私钥"));

    // 写入完整配置并启用。
    let (status, body) = request(
        router.clone(),
        "PUT",
        "/api/billing/payment-channels/alipay",
        Some(&cookie),
        Some(json!({
            "enabled": true,
            "app_id": "2021000000000000",
            "gateway": "",
            "notify_url": "https://aio.addzero.site/api/billing/alipay/notify",
            "return_url": "https://aio.addzero.site/",
            "seller_id": "",
            "public_key": public_pem,
            "private_key": private_pem,
        })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["enabled"], true);
    assert_eq!(body["data"]["has_private_key"], true);
    // 视图不回传私钥明文，且数据库只存密文。
    assert!(body["data"].get("private_key").is_none());
    let stored: String = sqlx::query_scalar(
        "SELECT private_key_ciphertext FROM billing_payment_channels WHERE tenant_id = $1",
    )
    .bind(&session.tenant_id)
    .fetch_one(&pool)
    .await?;
    assert_ne!(stored, private_pem);
    assert!(!stored.contains("BEGIN PRIVATE KEY"));

    // 读取渠道时同样只返回 has_private_key。
    let (status, body) = request(
        router.clone(),
        "GET",
        "/api/billing/payment-channels",
        Some(&cookie),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"][0]["has_private_key"], true);
    assert!(body["data"][0].get("private_key").is_none());

    // 创建支付宝订单会返回带签名的支付地址。
    let (status, body) = request(
        router.clone(),
        "POST",
        "/api/billing/orders",
        Some(&cookie),
        Some(json!({"amount_micros": 60_000_000, "provider": "alipay"})),
    )
    .await?;
    assert_eq!(status, StatusCode::CREATED);
    let order_id = body["data"]["id"].as_str().unwrap().to_owned();
    let pay_url = body["data"]["pay_url"].as_str().unwrap();
    assert!(pay_url.starts_with("https://openapi.alipay.com/gateway.do?"));
    assert!(pay_url.contains("sign_type=RSA2"));
    assert!(pay_url.contains("sign="));

    // 伪造签名必须被拒绝。
    let forged = vec![
        ("out_trade_no".to_owned(), order_id.clone()),
        ("trade_status".to_owned(), "TRADE_SUCCESS".to_owned()),
        ("app_id".to_owned(), "2021000000000000".to_owned()),
        ("total_amount".to_owned(), "60.00".to_owned()),
        ("sign_type".to_owned(), "RSA2".to_owned()),
        ("sign".to_owned(), STANDARD.encode([0_u8; 256])),
    ];
    assert!(service.settle_alipay_notification(&forged).await.is_err());

    // 合法通知入账；金额不匹配必须被拒绝。
    let mut signed = vec![
        ("out_trade_no".to_owned(), order_id.clone()),
        ("trade_status".to_owned(), "TRADE_SUCCESS".to_owned()),
        ("app_id".to_owned(), "2021000000000000".to_owned()),
        ("total_amount".to_owned(), "60.00".to_owned()),
        ("sign_type".to_owned(), "RSA2".to_owned()),
    ];
    let content = crate::service::billing::alipay_signing_content_for_test(&signed);
    let digest = Sha256::digest(content.as_bytes());
    let signature = private.sign(Pkcs1v15Sign::new::<Sha256>(), &digest)?;
    signed.push(("sign".to_owned(), STANDARD.encode(signature)));
    assert_eq!(
        service.settle_alipay_notification(&signed).await?,
        "success"
    );
    let balance: i64 =
        sqlx::query_scalar("SELECT balance_micros FROM billing_wallets WHERE tenant_id = $1")
            .bind(&session.tenant_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(balance, 60_000_000);
    // 重复通知幂等。
    assert_eq!(
        service.settle_alipay_notification(&signed).await?,
        "success"
    );
    let balance: i64 =
        sqlx::query_scalar("SELECT balance_micros FROM billing_wallets WHERE tenant_id = $1")
            .bind(&session.tenant_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(balance, 60_000_000);

    let mut wrong_amount = signed
        .iter()
        .filter(|(key, _)| key != "sign" && key != "sign_type")
        .cloned()
        .collect::<Vec<_>>();
    wrong_amount
        .iter_mut()
        .find(|(key, _)| key == "total_amount")
        .unwrap()
        .1 = "1.00".to_owned();
    let content = crate::service::billing::alipay_signing_content_for_test(&wrong_amount);
    let digest = Sha256::digest(content.as_bytes());
    let signature = private.sign(Pkcs1v15Sign::new::<Sha256>(), &digest)?;
    wrong_amount.push(("sign_type".to_owned(), "RSA2".to_owned()));
    wrong_amount.push(("sign".to_owned(), STANDARD.encode(signature)));
    assert!(
        service
            .settle_alipay_notification(&wrong_amount)
            .await
            .is_err()
    );

    Ok(())
}
