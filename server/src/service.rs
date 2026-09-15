use std::env;

use aio_plugin_identity_model::{LoginRequest, PasswordRequest, SessionView};
use anyhow::{Context as _, Result, ensure};
use axum::http::{HeaderMap, header};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, postgres::PgPoolOptions};

use crate::password;

#[path = "registration/mod.rs"]
mod registration;
pub(crate) use registration::RegistrationError;

const TENANT_PERMISSIONS: [&str; 5] = [
    "plugin:manage",
    "tenant:manage",
    "rbac:manage",
    "dictionary:manage",
    "file:manage",
];

const COOKIE_NAME: &str = "aio_session";
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS identity_users (
    id TEXT PRIMARY KEY,
    account TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS tenants (
    id TEXT PRIMARY KEY,
    label TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS auth_sessions (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    tenant_id TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS tenant_memberships (
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    display_name TEXT NOT NULL,
    PRIMARY KEY(tenant_id, user_id)
);
CREATE TABLE IF NOT EXISTS tenant_member_roles (
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    role_id TEXT NOT NULL,
    PRIMARY KEY(tenant_id, user_id, role_id)
);
CREATE TABLE IF NOT EXISTS role_permissions (
    tenant_id TEXT NOT NULL,
    role_id TEXT NOT NULL,
    permission TEXT NOT NULL,
    PRIMARY KEY(tenant_id, role_id, permission)
);
CREATE INDEX IF NOT EXISTS auth_sessions_expires_at_idx ON auth_sessions(expires_at);
"#;

#[derive(Clone, Debug)]
pub struct SessionContext {
    pub session_id: String,
    pub user_id: String,
    pub account: String,
    pub display_name: String,
    pub tenant_id: String,
    pub tenant_label: String,
    pub permissions: Vec<String>,
}

impl SessionContext {
    pub fn view(&self) -> SessionView {
        SessionView {
            user_id: self.user_id.clone(),
            account: self.account.clone(),
            display_name: self.display_name.clone(),
            tenant_id: self.tenant_id.clone(),
            tenant_label: self.tenant_label.clone(),
            permissions: self.permissions.clone(),
        }
    }
}

#[derive(Debug)]
pub struct LoginResult {
    pub session: SessionContext,
    pub cookie: String,
}

#[derive(Debug)]
pub struct IdentityService {
    pool: PgPool,
    secure_cookie: bool,
    password_min_length: usize,
    registration_slots: tokio::sync::Semaphore,
}

impl IdentityService {
    pub fn from_env() -> Result<Self> {
        let database_url = env::var("AIO_DATABASE_URL")
            .or_else(|_| env::var("AZ_AIO_DATABASE_URL"))
            .context("身份插件缺少 AIO_DATABASE_URL")?;
        let secure_cookie = env::var("AIO_SESSION_SECURE")
            .map(|value| value != "0" && value != "false")
            .unwrap_or(false);
        let password_min_length = env::var("AIO_PASSWORD_MIN_LENGTH")
            .ok()
            .map(|value| {
                value
                    .parse::<usize>()
                    .context("AIO_PASSWORD_MIN_LENGTH 必须是整数")
            })
            .transpose()?
            .unwrap_or(12);
        ensure!(
            (8..=128).contains(&password_min_length),
            "AIO_PASSWORD_MIN_LENGTH 必须在 8 到 128 之间"
        );
        Ok(Self {
            pool: PgPoolOptions::new()
                .max_connections(10)
                .connect_lazy(&database_url)
                .context("创建身份数据库连接池失败")?,
            secure_cookie,
            password_min_length,
            registration_slots: tokio::sync::Semaphore::new(4),
        })
    }

    pub async fn initialize(&self) -> Result<()> {
        sqlx::raw_sql(SCHEMA)
            .execute(&self.pool)
            .await
            .context("创建身份插件数据表失败")?;
        self.bootstrap().await
    }

    async fn bootstrap(&self) -> Result<()> {
        let count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM identity_users")
            .fetch_one(&self.pool)
            .await?;
        if count > 0 {
            return Ok(());
        }
        let account = env::var("AIO_BOOTSTRAP_ACCOUNT").unwrap_or_else(|_| "admin".to_owned());
        let password = match env::var("AIO_BOOTSTRAP_PASSWORD") {
            Ok(value) => value,
            Err(_) if env::var("AIO_ALLOW_INSECURE_BOOTSTRAP").as_deref() == Ok("1") => {
                "aio-dev-password".to_owned()
            }
            Err(_) => anyhow::bail!("首次启动必须配置 AIO_BOOTSTRAP_PASSWORD"),
        };
        self.validate_password(&password, "初始密码")?;
        let user_id = uuid::Uuid::new_v4().to_string();
        let password_hash = password::hash(&password)?;
        let mut transaction = self.pool.begin().await?;
        sqlx::query("INSERT INTO identity_users (id, account, display_name, password_hash) VALUES ($1, $2, $3, $4)")
            .bind(&user_id).bind(&account).bind("平台管理员").bind(password_hash)
            .execute(&mut *transaction).await?;
        sqlx::query(
            "INSERT INTO tenants (id, label) VALUES ('default', '默认租户') ON CONFLICT DO NOTHING",
        )
        .execute(&mut *transaction)
        .await?;
        sqlx::query("INSERT INTO tenant_memberships (tenant_id, user_id, display_name) VALUES ('default', $1, '平台管理员') ON CONFLICT DO NOTHING")
            .bind(&user_id).execute(&mut *transaction).await?;
        sqlx::query("INSERT INTO tenant_member_roles (tenant_id, user_id, role_id) VALUES ('default', $1, 'platform-admin') ON CONFLICT DO NOTHING")
            .bind(&user_id).execute(&mut *transaction).await?;
        for permission in TENANT_PERMISSIONS {
            sqlx::query("INSERT INTO role_permissions (tenant_id, role_id, permission) VALUES ('default', 'platform-admin', $1) ON CONFLICT DO NOTHING")
                .bind(permission).execute(&mut *transaction).await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    pub async fn login(&self, request: &LoginRequest) -> Result<Option<LoginResult>> {
        let row = sqlx::query_as::<_, (String, String, String)>(
            "SELECT id, display_name, password_hash FROM identity_users WHERE account = $1",
        )
        .bind(request.account.trim())
        .fetch_optional(&self.pool)
        .await?;
        let Some((user_id, display_name, password_hash)) = row else {
            return Ok(None);
        };
        let password = request.password.clone();
        let valid =
            tokio::task::spawn_blocking(move || password::verify(&password, &password_hash))
                .await
                .context("密码校验任务失败")?;
        if !valid {
            return Ok(None);
        }
        let (tenant_id, tenant_label) = sqlx::query_as::<_, (String, String)>(
            "SELECT memberships.tenant_id, tenants.label FROM tenant_memberships memberships JOIN tenants ON tenants.id = memberships.tenant_id WHERE memberships.user_id = $1 ORDER BY memberships.tenant_id LIMIT 1",
        )
        .bind(&user_id)
        .fetch_optional(&self.pool)
        .await?
        .context("账号没有可用租户")?;
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let session_id = session_id(&token);
        sqlx::query("DELETE FROM auth_sessions WHERE expires_at <= now()")
            .execute(&self.pool)
            .await?;
        sqlx::query("INSERT INTO auth_sessions (id, user_id, tenant_id, expires_at) VALUES ($1, $2, $3, now() + interval '7 days')")
            .bind(&session_id).bind(&user_id).bind(&tenant_id).execute(&self.pool).await?;
        let permissions = self.permissions(&tenant_id, &user_id).await?;
        Ok(Some(LoginResult {
            session: SessionContext {
                session_id,
                user_id,
                account: request.account.trim().to_owned(),
                display_name,
                tenant_id,
                tenant_label,
                permissions,
            },
            cookie: self.session_cookie(&token),
        }))
    }

    pub async fn authenticate(&self, headers: &HeaderMap) -> Result<Option<SessionContext>> {
        let Some(token) = cookie(headers, COOKIE_NAME) else {
            return Ok(None);
        };
        let id = session_id(token);
        let row = sqlx::query_as::<_, (String, String, String, String, String)>(
            "SELECT users.id, users.account, users.display_name, sessions.tenant_id, tenants.label FROM auth_sessions sessions JOIN identity_users users ON users.id = sessions.user_id JOIN tenants ON tenants.id = sessions.tenant_id WHERE sessions.id = $1 AND sessions.expires_at > now()",
        )
        .bind(&id)
        .fetch_optional(&self.pool)
        .await?;
        let Some((user_id, account, display_name, tenant_id, tenant_label)) = row else {
            return Ok(None);
        };
        let permissions = self.permissions(&tenant_id, &user_id).await?;
        Ok(Some(SessionContext {
            session_id: id,
            user_id,
            account,
            display_name,
            tenant_id,
            tenant_label,
            permissions,
        }))
    }

    pub async fn logout(&self, headers: &HeaderMap) -> Result<()> {
        if let Some(token) = cookie(headers, COOKIE_NAME) {
            sqlx::query("DELETE FROM auth_sessions WHERE id = $1")
                .bind(session_id(token))
                .execute(&self.pool)
                .await?;
        }
        Ok(())
    }

    pub async fn switch_tenant(&self, session: &SessionContext, tenant_id: &str) -> Result<bool> {
        let membership = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM tenant_memberships WHERE tenant_id = $1 AND user_id = $2)",
        )
        .bind(tenant_id)
        .bind(&session.user_id)
        .fetch_one(&self.pool)
        .await?;
        if !membership {
            return Ok(false);
        }
        let result = sqlx::query(
            "UPDATE auth_sessions SET tenant_id = $2 WHERE id = $1 AND user_id = $3 AND expires_at > now()",
        )
        .bind(&session.session_id)
        .bind(tenant_id)
        .bind(&session.user_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn create_user(
        &self,
        tenant_id: &str,
        account: &str,
        display_name: &str,
        password: &str,
    ) -> Result<String> {
        let account = account.trim();
        let display_name = display_name.trim();
        ensure!(!account.is_empty(), "账号不能为空");
        ensure!(
            account.chars().all(
                |character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
            ),
            "账号只能包含字母、数字、连字符和下划线"
        );
        ensure!(!display_name.is_empty(), "显示名称不能为空");
        self.validate_password(password, "初始密码")?;
        let password = password.to_owned();
        let password_hash = tokio::task::spawn_blocking(move || password::hash(&password))
            .await
            .context("密码摘要任务失败")??;
        let user_id = uuid::Uuid::new_v4().to_string();
        let mut transaction = self.pool.begin().await?;
        sqlx::query("INSERT INTO identity_users (id, account, display_name, password_hash) VALUES ($1, $2, $3, $4)")
            .bind(&user_id).bind(account).bind(display_name).bind(password_hash)
            .execute(&mut *transaction).await?;
        sqlx::query(
            "INSERT INTO tenant_memberships (tenant_id, user_id, display_name) VALUES ($1, $2, $3)",
        )
        .bind(tenant_id)
        .bind(&user_id)
        .bind(display_name)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(user_id)
    }

    pub async fn change_password(
        &self,
        session: &SessionContext,
        request: &PasswordRequest,
    ) -> Result<bool> {
        self.validate_password(&request.new_password, "新密码")?;
        let encoded = sqlx::query_scalar::<_, String>(
            "SELECT password_hash FROM identity_users WHERE id = $1",
        )
        .bind(&session.user_id)
        .fetch_one(&self.pool)
        .await?;
        let current_password = request.current_password.clone();
        let valid =
            tokio::task::spawn_blocking(move || password::verify(&current_password, &encoded))
                .await
                .context("密码校验任务失败")?;
        if !valid {
            return Ok(false);
        }
        let password = request.new_password.clone();
        let encoded = tokio::task::spawn_blocking(move || password::hash(&password))
            .await
            .context("密码摘要任务失败")??;
        sqlx::query("UPDATE identity_users SET password_hash = $2 WHERE id = $1")
            .bind(&session.user_id)
            .bind(encoded)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM auth_sessions WHERE user_id = $1 AND id <> $2")
            .bind(&session.user_id)
            .bind(&session.session_id)
            .execute(&self.pool)
            .await?;
        Ok(true)
    }

    async fn permissions(&self, tenant_id: &str, user_id: &str) -> Result<Vec<String>> {
        sqlx::query_scalar(
            "SELECT DISTINCT permissions.permission FROM tenant_member_roles roles JOIN role_permissions permissions ON permissions.tenant_id = roles.tenant_id AND permissions.role_id = roles.role_id WHERE roles.tenant_id = $1 AND roles.user_id = $2 ORDER BY permissions.permission",
        )
        .bind(tenant_id)
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }

    fn session_cookie(&self, token: &str) -> String {
        let secure = if self.secure_cookie { "; Secure" } else { "" };
        format!("{COOKIE_NAME}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age=604800{secure}")
    }

    pub fn expired_cookie(&self) -> String {
        let secure = if self.secure_cookie { "; Secure" } else { "" };
        format!("{COOKIE_NAME}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{secure}")
    }

    fn validate_password(&self, password: &str, label: &str) -> Result<()> {
        ensure!(
            password.chars().count() >= self.password_min_length,
            "{label}至少需要 {} 个字符",
            self.password_min_length
        );
        Ok(())
    }
}

fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|part| part.trim().split_once('='))
        .find_map(|(key, value)| (key == name).then_some(value))
}

fn session_id(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_named_cookie_without_accepting_prefixes() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            "other=1; aio_session=secret".parse().unwrap(),
        );
        assert_eq!(cookie(&headers, COOKIE_NAME), Some("secret"));
        assert_eq!(cookie(&headers, "session"), None);
    }

    #[test]
    fn hashes_session_token() {
        assert_ne!(session_id("secret"), "secret");
        assert_eq!(session_id("secret"), session_id("secret"));
    }
}
