# AIO 身份插件

独立提供账号密码登录、Argon2 密码摘要、HttpOnly 会话 Cookie、退出、修改密码和个人资料页面。首次启动必须提供 `AIO_BOOTSTRAP_PASSWORD`；仅本地开发可以显式设置 `AIO_ALLOW_INSECURE_BOOTSTRAP=1` 使用开发密码。

密码默认最短 12 个字符。部署方可以通过 `AIO_PASSWORD_MIN_LENGTH` 配置 8 到 128 之间的最短长度。

登录页提供「注册账号」弹窗。注册无需验证码、邮箱验证或审核，成功后自动登录并创建独立工作区，拥有该工作区的租户管理权限。注册请求不能指定已有租户、角色或权限，也不会获得平台发布者身份。接口为 `POST /api/auth/register`，接收 `account` 和 `password`；重复账号返回 409。
