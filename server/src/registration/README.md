# 账号注册

本机 PostgreSQL 验证：`AIO_IDENTITY_TEST_DATABASE_URL='postgresql://postgres@localhost/aio_identity_test?host=/tmp' cargo test --workspace -- --include-ignored`。测试自动创建并移除独立 schema，覆盖并发重名、错误输入、事务完整性、工作区隔离、会话恢复和重新登录。

公开注册不要求验证码、邮箱验证或审核，成功后直接建立登录会话。账号、独立租户、成员、租户管理员权限和会话在同一 PostgreSQL 事务提交；客户端不能指定租户或权限。遵循宿主密码最短长度策略，密码使用 Argon2 摘要。单实例最多同时处理 4 个注册请求，接口正文限制为 16 KiB。
