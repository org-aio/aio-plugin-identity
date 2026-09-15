use super::super::{IdentityService, SCHEMA};
use anyhow::Result;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use serde_json::{Value, json};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use std::{str::FromStr, sync::Arc};
use tower::ServiceExt;

async fn post(
    router: axum::Router,
    path: &str,
    body: Value,
) -> Result<(StatusCode, Option<String>, Value)> {
    let response = router
        .oneshot(
            Request::post(path)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))?,
        )
        .await?;
    let status = response.status();
    let cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .map(|v| v.to_str().unwrap().to_owned());
    let bytes = to_bytes(response.into_body(), 100_000).await?;
    Ok((
        status,
        cookie,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    ))
}

#[tokio::test]
#[ignore = "需要 AIO_IDENTITY_TEST_DATABASE_URL 指向本机独立 aio_identity_test 数据库"]
async fn registration_is_atomic_isolated_and_immediately_usable() -> Result<()> {
    let url = std::env::var("AIO_IDENTITY_TEST_DATABASE_URL")?;
    let options = PgConnectOptions::from_str(&url)?;
    anyhow::ensure!(
        matches!(options.get_host(), "localhost" | "127.0.0.1" | "/tmp")
            && options.get_database() == Some("aio_identity_test"),
        "只允许独立本机测试库"
    );
    let root = PgPoolOptions::new().connect_with(options.clone()).await?;
    let schema = format!("registration_{}", uuid::Uuid::new_v4().simple());
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
    let router = crate::routes::router(service.clone());
    let password = "registration-test-password";
    let (status, cookie, body) = post(
        router.clone(),
        "/api/auth/register",
        json!({"account":" first_user ","password":password}),
    )
    .await?;
    assert_eq!(status, StatusCode::CREATED);
    let cookie = cookie.unwrap();
    assert!(
        cookie.contains("HttpOnly") && cookie.contains("Secure") && cookie.contains("SameSite=Lax")
    );
    let session = &body["data"];
    assert_eq!(session["account"], "first_user");
    assert_ne!(session["tenant_id"], "default");
    let hash: String =
        sqlx::query_scalar("SELECT password_hash FROM identity_users WHERE account='first_user'")
            .fetch_one(&pool)
            .await?;
    assert_ne!(hash, password);
    assert!(crate::password::verify(password, &hash));
    let duplicate = post(
        router.clone(),
        "/api/auth/register",
        json!({"account":"first_user","password":password}),
    )
    .await?;
    assert_eq!(duplicate.0, StatusCode::CONFLICT);
    assert!(duplicate.1.is_none());
    assert_eq!(duplicate.2["error"], "账号已被使用");
    for body in [
        json!({"account":"bad user","password":password}),
        json!({"account":"weak","password":"short"}),
    ] {
        assert_eq!(
            post(router.clone(), "/api/auth/register", body).await?.0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(post(router.clone(), "/api/auth/register", json!({"account":"forged","password":password,"tenant_id":"default","permissions":["*"]})).await?.0, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        post(
            router.clone(),
            "/api/auth/register",
            json!({"account":"large","password":"x".repeat(20_000)})
        )
        .await?
        .0,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    let (first, second) = tokio::join!(
        post(
            router.clone(),
            "/api/auth/register",
            json!({"account":"second_user","password":password})
        ),
        post(
            router.clone(),
            "/api/auth/register",
            json!({"account":"second_user","password":password})
        )
    );
    let first = first?;
    let second = second?;
    assert!(
        (first.0 == StatusCode::CREATED && second.0 == StatusCode::CONFLICT)
            || (second.0 == StatusCode::CREATED && first.0 == StatusCode::CONFLICT)
    );
    let second = if first.0 == StatusCode::CREATED {
        first.2
    } else {
        second.2
    };
    assert_ne!(session["tenant_id"], second["data"]["tenant_id"]);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM identity_users")
            .fetch_one(&pool)
            .await?,
        2
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM tenants")
            .fetch_one(&pool)
            .await?,
        2
    );
    let restored = IdentityService {
        pool: pool.clone(),
        secure_cookie: true,
        password_min_length: 12,
        registration_slots: tokio::sync::Semaphore::new(4),
    };
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(header::COOKIE, cookie.split(';').next().unwrap().parse()?);
    let authenticated = restored.authenticate(&headers).await?.unwrap();
    assert!(authenticated.permissions.contains(&"plugin:manage".into()));
    assert!(
        !restored
            .switch_tenant(
                &authenticated,
                second["data"]["tenant_id"].as_str().unwrap()
            )
            .await?
    );
    restored.logout(&headers).await?;
    assert!(restored.authenticate(&headers).await?.is_none());
    assert_eq!(
        post(
            router,
            "/api/auth/login",
            json!({"account":"first_user","password":password})
        )
        .await?
        .0,
        StatusCode::OK
    );
    drop(service);
    pool.close().await;
    sqlx::raw_sql(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&root)
        .await?;
    Ok(())
}
