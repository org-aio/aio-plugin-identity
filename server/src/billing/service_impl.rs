use aio_plugin_identity_model::{
    LedgerEntry, LedgerPage, PlanUsage, PlanView, RechargeOrderView, RechargeRequest,
    SubscribeRequest, SubscriptionView, UsageGrantView, WalletView,
};
use anyhow::{Context as _, Result};
use sqlx::Row;
use uuid::Uuid;

use super::BillingError;
use crate::{IdentityService, SessionContext};

const DEFAULT_CURRENCY: &str = "USD";
const LEDGER_PAGE_LIMIT: i64 = 20;

/// 单次计费上报；插件只传资源用量，价格由平台价目决定。
pub struct MeterCommand {
    pub tenant_id: String,
    pub user_id: String,
    pub source_id: String,
    pub resource: String,
    pub quantity: i64,
    pub unit_price_micros: i64,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeterResult {
    pub amount_micros: i64,
    pub grant_consumed: i64,
    pub balance_charged_micros: i64,
    pub balance_after_micros: i64,
    /// 命中幂等键时为 `true`，表示本次未重复扣费。
    pub duplicate: bool,
}

impl IdentityService {
    pub async fn wallet(&self, session: &SessionContext) -> Result<WalletView> {
        self.ensure_wallet(&session.tenant_id).await?;
        let row = sqlx::query(
            "SELECT currency, balance_micros FROM billing_wallets WHERE tenant_id = $1",
        )
        .bind(&session.tenant_id)
        .fetch_one(&self.pool)
        .await
        .context("读取钱包余额失败")?;
        let currency: String = row.get("currency");
        let balance_micros: i64 = row.get("balance_micros");
        Ok(WalletView {
            currency,
            balance_micros,
            subscription: self.active_subscription(&session.tenant_id).await?,
            grants: self.active_grants(&session.tenant_id).await?,
        })
    }

    pub async fn ledger(
        &self,
        session: &SessionContext,
        cursor: Option<&str>,
    ) -> Result<LedgerPage> {
        let rows = match cursor {
            Some(cursor) => {
                sqlx::query(
                    r#"SELECT id, kind, amount_micros, balance_after_micros, description,
                              to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS created_at
                       FROM billing_ledger
                       WHERE tenant_id = $1
                         AND (created_at, id) < (
                             SELECT created_at, id FROM billing_ledger WHERE tenant_id = $1 AND id = $2
                         )
                       ORDER BY created_at DESC, id DESC
                       LIMIT $3"#,
                )
                .bind(&session.tenant_id)
                .bind(cursor)
                .bind(LEDGER_PAGE_LIMIT + 1)
                .fetch_all(&self.pool)
                .await
            }
            None => {
                sqlx::query(
                    r#"SELECT id, kind, amount_micros, balance_after_micros, description,
                              to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS created_at
                       FROM billing_ledger
                       WHERE tenant_id = $1
                       ORDER BY created_at DESC, id DESC
                       LIMIT $2"#,
                )
                .bind(&session.tenant_id)
                .bind(LEDGER_PAGE_LIMIT + 1)
                .fetch_all(&self.pool)
                .await
            }
        }
        .context("读取账本失败")?;
        let mut entries = rows
            .into_iter()
            .map(|row| LedgerEntry {
                id: row.get("id"),
                kind: row.get("kind"),
                amount_micros: row.get("amount_micros"),
                balance_after_micros: row.get("balance_after_micros"),
                description: row.get("description"),
                created_at: row.get("created_at"),
            })
            .collect::<Vec<_>>();
        let has_more = entries.len() as i64 > LEDGER_PAGE_LIMIT;
        if has_more {
            entries.truncate(LEDGER_PAGE_LIMIT as usize);
        }
        Ok(LedgerPage {
            next_cursor: has_more
                .then(|| entries.last().map(|entry| entry.id.clone()))
                .flatten(),
            entries,
        })
    }

    /// 创建充值订单。真实支付渠道接入前，只创建 `pending` 订单。
    pub async fn create_recharge_order(
        &self,
        session: &SessionContext,
        request: &RechargeRequest,
    ) -> Result<RechargeOrderView> {
        if request.amount_micros <= 0 {
            return Err(BillingError::Invalid("充值金额必须大于 0".into()).into());
        }
        let provider = request.provider.trim();
        if provider.is_empty()
            || provider.len() > 32
            || !provider
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
        {
            return Err(BillingError::Invalid("充值渠道无效".into()).into());
        }
        self.ensure_wallet(&session.tenant_id).await?;
        let id = Uuid::new_v4().to_string();
        let row = sqlx::query(
            r#"INSERT INTO billing_orders
                   (id, tenant_id, amount_micros, currency, provider, status, created_by)
               VALUES ($1, $2, $3, $4, $5, 'pending', $6)
               RETURNING id, amount_micros, currency, provider, status,
                         to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS created_at"#,
        )
        .bind(&id)
        .bind(&session.tenant_id)
        .bind(request.amount_micros)
        .bind(DEFAULT_CURRENCY)
        .bind(provider)
        .bind(&session.user_id)
        .fetch_one(&self.pool)
        .await
        .context("创建充值订单失败")?;
        Ok(RechargeOrderView {
            id: row.get("id"),
            amount_micros: row.get("amount_micros"),
            currency: row.get("currency"),
            provider: row.get("provider"),
            status: row.get("status"),
            created_at: row.get("created_at"),
        })
    }

    /// 确认充值到账。渠道回调接入前，这里只用于人工确认，必须显式传入订单 ID。
    pub async fn confirm_recharge_order(
        &self,
        session: &SessionContext,
        order_id: &str,
    ) -> Result<WalletView> {
        let mut tx = self.pool.begin().await.context("开始充值确认事务失败")?;
        let row = sqlx::query(
            r#"SELECT amount_micros, currency, status FROM billing_orders
               WHERE id = $1 AND tenant_id = $2
               FOR UPDATE"#,
        )
        .bind(order_id)
        .bind(&session.tenant_id)
        .fetch_optional(&mut *tx)
        .await
        .context("锁定充值订单失败")?;
        let Some(row) = row else {
            return Err(BillingError::NotFound("充值订单不存在".into()).into());
        };
        let status: String = row.get("status");
        if status == "paid" {
            // 幂等：重复确认直接返回当前钱包。
            tx.rollback().await?;
            return self.wallet(session).await;
        }
        if status != "pending" {
            return Err(BillingError::Invalid(format!("订单状态为 {status}，无法确认")).into());
        }
        let amount_micros: i64 = row.get("amount_micros");
        self.ensure_wallet_in(&mut tx, &session.tenant_id).await?;
        let balance_after = self
            .credit_in(
                &mut tx,
                LedgerWrite {
                    tenant_id: &session.tenant_id,
                    amount_micros,
                    kind: "recharge",
                    reference: Some(order_id),
                    description: &format!("充值 {}", format_micros(amount_micros)),
                    created_by: &session.user_id,
                },
            )
            .await?;
        sqlx::query("UPDATE billing_orders SET status = 'paid', paid_at = now() WHERE id = $1")
            .bind(order_id)
            .execute(&mut *tx)
            .await
            .context("更新充值订单状态失败")?;
        tx.commit().await.context("提交充值确认事务失败")?;
        debug_assert!(balance_after >= amount_micros);
        self.wallet(session).await
    }

    /// 订阅或切换套餐；新周期立即生效，旧订阅标记为 `canceled`。
    pub async fn subscribe(
        &self,
        session: &SessionContext,
        request: &SubscribeRequest,
    ) -> Result<WalletView> {
        let plan = sqlx::query(
            r#"SELECT id, code, title, price_micros, currency, interval, included_usage, features
               FROM billing_plans WHERE code = $1 AND active"#,
        )
        .bind(request.plan_code.trim())
        .fetch_optional(&self.pool)
        .await
        .context("读取套餐失败")?;
        let Some(plan) = plan else {
            return Err(BillingError::NotFound("套餐不存在".into()).into());
        };
        let plan_id: String = plan.get("id");
        let included_usage: serde_json::Value = plan.get("included_usage");
        let mut tx = self.pool.begin().await.context("开始订阅事务失败")?;
        sqlx::query(
            "UPDATE billing_subscriptions SET status = 'canceled' WHERE tenant_id = $1 AND status = 'active'",
        )
        .bind(&session.tenant_id)
        .execute(&mut *tx)
        .await
        .context("取消旧订阅失败")?;
        let subscription_id = Uuid::new_v4().to_string();
        let row = sqlx::query(
            r#"INSERT INTO billing_subscriptions
                   (id, tenant_id, plan_id, status, period_start, period_end, auto_renew)
               VALUES ($1, $2, $3, 'active', now(), now() + interval '1 month', true)
               RETURNING to_char(period_start AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS period_start,
                         to_char(period_end AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS period_end"#,
        )
        .bind(&subscription_id)
        .bind(&session.tenant_id)
        .bind(&plan_id)
        .fetch_one(&mut *tx)
        .await
        .context("创建订阅失败")?;
        let period_start: String = row.get("period_start");
        let period_end: String = row.get("period_end");
        if let Some(map) = included_usage.as_object() {
            for (resource, quantity) in map {
                let Some(quantity) = quantity.as_i64() else {
                    continue;
                };
                sqlx::query(
                    r#"INSERT INTO billing_usage_grants
                           (id, tenant_id, subscription_id, resource, quantity, consumed, period_start, period_end)
                       VALUES ($1, $2, $3, $4, $5, 0, now(), now() + interval '1 month')"#,
                )
                .bind(Uuid::new_v4().to_string())
                .bind(&session.tenant_id)
                .bind(&subscription_id)
                .bind(resource)
                .bind(quantity)
                .execute(&mut *tx)
                .await
                .context("发放套餐额度失败")?;
            }
        }
        tx.commit().await.context("提交订阅事务失败")?;
        debug_assert!(period_start < period_end);
        self.wallet(session).await
    }

    /// 内部计费用量上报：先扣当前周期套餐额度，再扣钱包余额。
    pub async fn meter(&self, command: MeterCommand) -> Result<MeterResult> {
        if command.quantity <= 0 {
            return Err(BillingError::Invalid("用量必须大于 0".into()).into());
        }
        if command.unit_price_micros < 0 {
            return Err(BillingError::Invalid("单价不能为负".into()).into());
        }
        let amount_micros = command
            .quantity
            .checked_mul(command.unit_price_micros)
            .context("用量金额超出范围")?;
        let mut tx = self.pool.begin().await.context("开始计费事务失败")?;
        self.ensure_wallet_in(&mut tx, &command.tenant_id).await?;
        let existing = sqlx::query(
            r#"SELECT amount_micros FROM billing_usage_events
               WHERE tenant_id = $1 AND source_id = $2 AND idempotency_key = $3"#,
        )
        .bind(&command.tenant_id)
        .bind(&command.source_id)
        .bind(&command.idempotency_key)
        .fetch_optional(&mut *tx)
        .await
        .context("检查计费幂等键失败")?;
        if let Some(existing) = existing {
            let balance_after = self.balance_in(&mut tx, &command.tenant_id).await?;
            tx.rollback().await?;
            return Ok(MeterResult {
                amount_micros: existing.get("amount_micros"),
                grant_consumed: 0,
                balance_charged_micros: 0,
                balance_after_micros: balance_after,
                duplicate: true,
            });
        }
        let grants = sqlx::query(
            r#"SELECT id, quantity, consumed FROM billing_usage_grants
               WHERE tenant_id = $1 AND resource = $2 AND period_end > now() AND consumed < quantity
               ORDER BY period_end, id
               FOR UPDATE"#,
        )
        .bind(&command.tenant_id)
        .bind(&command.resource)
        .fetch_all(&mut *tx)
        .await
        .context("锁定套餐额度失败")?;
        let mut remaining = command.quantity;
        let mut grant_consumed = 0_i64;
        for grant in grants {
            if remaining == 0 {
                break;
            }
            let available: i64 = grant.get::<i64, _>("quantity") - grant.get::<i64, _>("consumed");
            if available <= 0 {
                continue;
            }
            let take = available.min(remaining);
            sqlx::query("UPDATE billing_usage_grants SET consumed = consumed + $2 WHERE id = $1")
                .bind(grant.get::<String, _>("id"))
                .bind(take)
                .execute(&mut *tx)
                .await
                .context("扣减套餐额度失败")?;
            remaining -= take;
            grant_consumed += take;
        }
        let balance_charged_micros = remaining
            .checked_mul(command.unit_price_micros)
            .context("余额扣费金额超出范围")?;
        if balance_charged_micros > 0 {
            let balance = self.balance_in(&mut tx, &command.tenant_id).await?;
            if balance < balance_charged_micros {
                return Err(BillingError::InsufficientFunds.into());
            }
        }
        let usage_id = Uuid::new_v4().to_string();
        sqlx::query(
            r#"INSERT INTO billing_usage_events
                   (id, tenant_id, user_id, source_id, resource, quantity, unit_price_micros,
                    amount_micros, idempotency_key, occurred_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, now())"#,
        )
        .bind(&usage_id)
        .bind(&command.tenant_id)
        .bind(&command.user_id)
        .bind(&command.source_id)
        .bind(&command.resource)
        .bind(command.quantity)
        .bind(command.unit_price_micros)
        .bind(amount_micros)
        .bind(&command.idempotency_key)
        .execute(&mut *tx)
        .await
        .context("写入用量事件失败")?;
        let balance_after_micros = if balance_charged_micros > 0 {
            self.debit_in(
                &mut tx,
                LedgerWrite {
                    tenant_id: &command.tenant_id,
                    amount_micros: balance_charged_micros,
                    kind: "usage",
                    reference: Some(&usage_id),
                    description: &format!(
                        "{} 用量 {} {}",
                        command.source_id, command.quantity, command.resource
                    ),
                    created_by: &command.user_id,
                },
            )
            .await?
        } else {
            self.balance_in(&mut tx, &command.tenant_id).await?
        };
        tx.commit().await.context("提交计费事务失败")?;
        Ok(MeterResult {
            amount_micros,
            grant_consumed,
            balance_charged_micros,
            balance_after_micros,
            duplicate: false,
        })
    }

    async fn active_subscription(&self, tenant_id: &str) -> Result<Option<SubscriptionView>> {
        let row = sqlx::query(
            r#"SELECT plans.code, plans.title, plans.price_micros, plans.currency, plans.interval,
                      plans.included_usage, plans.features,
                      subscriptions.status,
                      to_char(subscriptions.period_start AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS period_start,
                      to_char(subscriptions.period_end AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS period_end,
                      subscriptions.auto_renew
               FROM billing_subscriptions subscriptions
               JOIN billing_plans plans ON plans.id = subscriptions.plan_id
               WHERE subscriptions.tenant_id = $1 AND subscriptions.status = 'active'
               ORDER BY subscriptions.period_end DESC
               LIMIT 1"#,
        )
        .bind(tenant_id)
        .fetch_optional(&self.pool)
        .await
        .context("读取订阅失败")?;
        Ok(row.map(|row| SubscriptionView {
            plan: PlanView {
                code: row.get("code"),
                title: row.get("title"),
                price_micros: row.get("price_micros"),
                currency: row.get("currency"),
                interval: row.get("interval"),
                included_usage: json_usage(row.get("included_usage")),
                features: json_features(row.get("features")),
            },
            status: row.get("status"),
            period_start: row.get("period_start"),
            period_end: row.get("period_end"),
            auto_renew: row.get("auto_renew"),
        }))
    }

    async fn active_grants(&self, tenant_id: &str) -> Result<Vec<UsageGrantView>> {
        let rows = sqlx::query(
            r#"SELECT resource, quantity, consumed,
                      to_char(period_end AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS period_end
               FROM billing_usage_grants
               WHERE tenant_id = $1 AND period_end > now()
               ORDER BY period_end, resource"#,
        )
        .bind(tenant_id)
        .fetch_all(&self.pool)
        .await
        .context("读取套餐额度失败")?;
        Ok(rows
            .into_iter()
            .map(|row| UsageGrantView {
                resource: row.get("resource"),
                quantity: row.get("quantity"),
                consumed: row.get("consumed"),
                period_end: row.get("period_end"),
            })
            .collect())
    }

    async fn ensure_wallet(&self, tenant_id: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO billing_wallets (tenant_id, currency) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        )
        .bind(tenant_id)
        .bind(DEFAULT_CURRENCY)
        .execute(&self.pool)
        .await
        .context("创建钱包失败")?;
        Ok(())
    }

    async fn ensure_wallet_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO billing_wallets (tenant_id, currency) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        )
        .bind(tenant_id)
        .bind(DEFAULT_CURRENCY)
        .execute(&mut **tx)
        .await
        .context("创建钱包失败")?;
        Ok(())
    }

    async fn balance_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: &str,
    ) -> Result<i64> {
        sqlx::query_scalar("SELECT balance_micros FROM billing_wallets WHERE tenant_id = $1")
            .bind(tenant_id)
            .fetch_one(&mut **tx)
            .await
            .context("读取钱包余额失败")
    }

    async fn credit_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        write: LedgerWrite<'_>,
    ) -> Result<i64> {
        let balance_after = sqlx::query_scalar::<_, i64>(
            "UPDATE billing_wallets SET balance_micros = balance_micros + $2, updated_at = now() WHERE tenant_id = $1 RETURNING balance_micros",
        )
        .bind(write.tenant_id)
        .bind(write.amount_micros)
        .fetch_one(&mut **tx)
        .await
        .context("更新钱包余额失败")?;
        self.write_ledger_in(
            tx,
            LedgerWrite {
                amount_micros: write.amount_micros,
                ..write
            },
            balance_after,
        )
        .await?;
        Ok(balance_after)
    }

    async fn debit_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        write: LedgerWrite<'_>,
    ) -> Result<i64> {
        let balance_after = sqlx::query_scalar::<_, i64>(
            "UPDATE billing_wallets SET balance_micros = balance_micros - $2, updated_at = now() WHERE tenant_id = $1 RETURNING balance_micros",
        )
        .bind(write.tenant_id)
        .bind(write.amount_micros)
        .fetch_one(&mut **tx)
        .await
        .context("扣减钱包余额失败")?;
        self.write_ledger_in(
            tx,
            LedgerWrite {
                amount_micros: -write.amount_micros,
                ..write
            },
            balance_after,
        )
        .await?;
        Ok(balance_after)
    }

    async fn write_ledger_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        write: LedgerWrite<'_>,
        balance_after_micros: i64,
    ) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO billing_ledger
                   (id, tenant_id, kind, amount_micros, balance_after_micros, reference, description, created_by)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"#,
        )
        .bind(Uuid::new_v4().to_string())
        .bind(write.tenant_id)
        .bind(write.kind)
        .bind(write.amount_micros)
        .bind(balance_after_micros)
        .bind(write.reference)
        .bind(write.description)
        .bind(write.created_by)
        .execute(&mut **tx)
        .await
        .context("写入账本失败")?;
        Ok(())
    }
}

/// 账本写入参数，避免长参数列表。
struct LedgerWrite<'a> {
    tenant_id: &'a str,
    amount_micros: i64,
    kind: &'a str,
    reference: Option<&'a str>,
    description: &'a str,
    created_by: &'a str,
}

/// 金额格式化：微单位转成人类可读字符串，仅用于账本描述。
fn format_micros(amount_micros: i64) -> String {
    let sign = if amount_micros < 0 { "-" } else { "" };
    let amount = amount_micros.unsigned_abs();
    format!("{sign}{}.{:06}", amount / 1_000_000, amount % 1_000_000)
}

fn json_usage(value: serde_json::Value) -> Vec<PlanUsage> {
    value
        .as_object()
        .map(|map| {
            map.iter()
                .filter_map(|(resource, quantity)| {
                    quantity.as_i64().map(|quantity| PlanUsage {
                        resource: resource.clone(),
                        quantity,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn json_features(value: serde_json::Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_micros_without_floating_point() {
        assert_eq!(format_micros(60_000_000), "60.000000");
        assert_eq!(format_micros(1_500_000), "1.500000");
        assert_eq!(format_micros(1), "0.000001");
        assert_eq!(format_micros(-2_500_000), "-2.500000");
    }

    #[test]
    fn parses_plan_usage_and_features() {
        let usage = json_usage(serde_json::json!({ "agent_tokens": 20_000_000 }));
        assert_eq!(
            usage,
            vec![PlanUsage {
                resource: "agent_tokens".into(),
                quantity: 20_000_000
            }]
        );
        let features = json_features(serde_json::json!(["agent", "memory"]));
        assert_eq!(features, vec!["agent".to_owned(), "memory".to_owned()]);
        assert!(json_usage(serde_json::json!(null)).is_empty());
        assert!(json_features(serde_json::json!({})).is_empty());
    }
}
