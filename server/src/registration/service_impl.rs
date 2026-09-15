use super::{
    super::{
        IdentityService, LoginResult, SessionContext, TENANT_PERMISSIONS, password, session_id,
    },
    RegistrationError,
};
use aio_plugin_identity_model::RegisterRequest;
use anyhow::{Context, Result};
use uuid::Uuid;

impl IdentityService {
    pub async fn register_account(&self, request: &RegisterRequest) -> Result<LoginResult> {
        let _permit = self
            .registration_slots
            .try_acquire()
            .map_err(|_| RegistrationError::Busy)?;
        let account = request.account.trim();
        if account.is_empty()
            || account.len() > 64
            || !account
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c))
        {
            return Err(RegistrationError::Invalid(
                "账号需要 1 到 64 位字母、数字、连字符或下划线".into(),
            )
            .into());
        }
        self.validate_password(&request.password, "密码")
            .map_err(|e| RegistrationError::Invalid(e.to_string()))?;
        if request.password.chars().count() > 128 {
            return Err(RegistrationError::Invalid("密码不能超过 128 个字符".into()).into());
        }
        let password = request.password.clone();
        let hash = tokio::task::spawn_blocking(move || password::hash(&password))
            .await
            .context("注册密码摘要任务失败")??;
        let user_id = Uuid::new_v4().to_string();
        let tenant_id = Uuid::new_v4().to_string();
        let tenant_label = format!("{account} 的工作区");
        let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let session_id = session_id(&token);
        let mut tx = self.pool.begin().await?;
        let inserted = sqlx::query("INSERT INTO identity_users (id, account, display_name, password_hash) VALUES ($1, $2, $2, $3) ON CONFLICT (account) DO NOTHING")
            .bind(&user_id).bind(account).bind(hash).execute(&mut *tx).await?;
        if inserted.rows_affected() != 1 {
            return Err(RegistrationError::AccountTaken.into());
        }
        sqlx::query("INSERT INTO tenants (id, label) VALUES ($1, $2)")
            .bind(&tenant_id)
            .bind(&tenant_label)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO tenant_memberships (tenant_id, user_id, display_name) VALUES ($1, $2, $3)",
        )
        .bind(&tenant_id)
        .bind(&user_id)
        .bind(account)
        .execute(&mut *tx)
        .await?;
        sqlx::query("INSERT INTO tenant_member_roles (tenant_id, user_id, role_id) VALUES ($1, $2, 'tenant-admin')")
            .bind(&tenant_id).bind(&user_id).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO role_permissions (tenant_id, role_id, permission) SELECT $1, 'tenant-admin', unnest($2::TEXT[])")
            .bind(&tenant_id).bind(TENANT_PERMISSIONS.as_slice()).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO role_permissions (tenant_id, role_id, permission) VALUES ($1, 'member', 'workspace:view')")
            .bind(&tenant_id).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO auth_sessions (id, user_id, tenant_id, expires_at) VALUES ($1, $2, $3, now() + interval '7 days')")
            .bind(&session_id).bind(&user_id).bind(&tenant_id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(LoginResult {
            cookie: self.session_cookie(&token),
            session: SessionContext {
                session_id,
                user_id,
                account: account.into(),
                display_name: account.into(),
                tenant_id,
                tenant_label,
                permissions: TENANT_PERMISSIONS.iter().map(|p| (*p).into()).collect(),
            },
        })
    }
}
