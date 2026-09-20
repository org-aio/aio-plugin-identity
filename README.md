# AIO 身份插件 / AIO Identity Plugin

独立提供账号密码登录、Argon2 密码摘要、HttpOnly 会话 Cookie、退出、修改密码和个人资料页面。首次启动必须提供 `AIO_BOOTSTRAP_PASSWORD`；仅本地开发可以显式设置 `AIO_ALLOW_INSECURE_BOOTSTRAP=1` 使用开发密码。

Standalone account/password login, Argon2 password hashing, HttpOnly session cookies, sign-out, password change and profile pages. `AIO_BOOTSTRAP_PASSWORD` must be provided on first start; local development only may explicitly set `AIO_ALLOW_INSECURE_BOOTSTRAP=1` to use a development password.

密码默认最短 12 个字符。部署方可以通过 `AIO_PASSWORD_MIN_LENGTH` 配置 8 到 128 之间的最短长度。

Passwords are at least 12 characters by default. Deployers can configure a minimum length between 8 and 128 via `AIO_PASSWORD_MIN_LENGTH`.

登录页提供「注册账号」弹窗。注册无需验证码、邮箱验证或审核，成功后自动登录并创建独立工作区，拥有该工作区的租户管理权限。注册请求不能指定已有租户、角色或权限，也不会获得平台发布者身份。接口为 `POST /api/auth/register`，接收 `account` 和 `password`；重复账号返回 409。

The login page provides a “Register account” dialog. Registration requires no captcha, email verification or review; on success it auto-logs-in and creates an isolated workspace with tenant-admin rights over it. A registration request cannot target an existing tenant, role or permission, and grants no platform publisher identity. The endpoint is `POST /api/auth/register`, accepting `account` and `password`; duplicate accounts return 409.

## 钱包与计费 / Wallet and Billing

个人资料页包含租户钱包：余额、当前套餐、周期额度、充值、订阅和最近账本。金额一律以微单位整数（1e-6 货币单位）存储，账本是唯一事实来源，`billing_wallets.balance_micros` 只是账本求和的快照。

The profile page includes the tenant wallet: balance, current plan, period grants, recharge, subscription and recent ledger. Amounts are stored as integer micro-units (1e-6 currency units); the ledger is the source of truth and `billing_wallets.balance_micros` is only a materialized snapshot.

| 方法 / Method | 路径 / Path | 说明 / Description |
| --- | --- | --- |
| `GET` | `/api/billing/wallet` | 当前租户余额、套餐与周期额度 / Tenant balance, plan and grants |
| `GET` | `/api/billing/ledger?cursor=` | 账本分页 / Paged ledger |
| `POST` | `/api/billing/orders` | 创建充值订单 / Create a recharge order |
| `POST` | `/api/billing/orders/{id}/confirm` | 确认充值到账 / Confirm recharge settlement |
| `POST` | `/api/billing/subscription` | 订阅或切换套餐 / Subscribe or switch plan |

充值、订阅需要 `billing:manage` 权限；余额与账本对所有租户成员可见。重复确认订单和重复用量上报都按幂等键去重。内置 `agent_pro_60` 套餐为 60 美元 / 月、含 20,000,000 `agent_tokens`，额度耗尽后回落到钱包按量计费。

Recharge and subscription require the `billing:manage` permission; balance and ledger are visible to all tenant members. Repeated order confirmations and usage reports are de-duplicated by idempotency keys. The built-in `agent_pro_60` plan costs 60 USD/month with 20,000,000 `agent_tokens`; once grants are exhausted, usage falls back to the wallet balance.
