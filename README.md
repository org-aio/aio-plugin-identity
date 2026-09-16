# AIO 身份插件 / AIO Identity Plugin

独立提供账号密码登录、Argon2 密码摘要、HttpOnly 会话 Cookie、退出、修改密码和个人资料页面。首次启动必须提供 `AIO_BOOTSTRAP_PASSWORD`；仅本地开发可以显式设置 `AIO_ALLOW_INSECURE_BOOTSTRAP=1` 使用开发密码。

Standalone account/password login, Argon2 password hashing, HttpOnly session cookies, sign-out, password change and profile pages. `AIO_BOOTSTRAP_PASSWORD` must be provided on first start; local development only may explicitly set `AIO_ALLOW_INSECURE_BOOTSTRAP=1` to use a development password.

密码默认最短 12 个字符。部署方可以通过 `AIO_PASSWORD_MIN_LENGTH` 配置 8 到 128 之间的最短长度。

Passwords are at least 12 characters by default. Deployers can configure a minimum length between 8 and 128 via `AIO_PASSWORD_MIN_LENGTH`.

登录页提供「注册账号」弹窗。注册无需验证码、邮箱验证或审核，成功后自动登录并创建独立工作区，拥有该工作区的租户管理权限。注册请求不能指定已有租户、角色或权限，也不会获得平台发布者身份。接口为 `POST /api/auth/register`，接收 `account` 和 `password`；重复账号返回 409。

The login page provides a “Register account” dialog. Registration requires no captcha, email verification or review; on success it auto-logs-in and creates an isolated workspace with tenant-admin rights over it. A registration request cannot target an existing tenant, role or permission, and grants no platform publisher identity. The endpoint is `POST /api/auth/register`, accepting `account` and `password`; duplicate accounts return 409.
