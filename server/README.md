# 身份服务

提供账号密码验证、会话 Cookie、退出和密码修改接口。

宿主按代管理服务时，在初始化阶段调用 `IdentityService::from_env()` 和 `initialize()`；初始化失败或停止时调用 `shutdown().await` 关闭本次连接池。关闭不删除会话和业务记录，下一代必须创建新实例，不能复用已经关闭的 Service。
