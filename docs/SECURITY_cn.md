# 安全

> [English](SECURITY.md) | 简体中文

## 安全状态

本文描述当前 `0.1.0 / preview_epoch_0` 已实现控制和强制部署边界，不代表公开 `1.0.0 / E0001` RC 或外部安全审计已经通过。

## 信任边界

浏览器输入、上传文件、REST/MCP 客户端、模型输出、Provider 响应、反向代理 header、备份介质和更新 metadata，在其责任边界完成校验前都属于不可信输入。

Desktop 的无密码本地入口只在可信 OS 账号内确认操作者；Server 才是具有完整账号安全边界的运行形态。

## Server 身份与会话

- 密码保存为 Argon2id PHC hash；管理 API 永不返回明文密码或 hash。
- Session/外部 Token 明文只在协议必需时出现，PostgreSQL 只保存不可复用 digest。
- 浏览器 Session 使用 HttpOnly、SameSite Cookie 和会话派生 CSRF；生产环境必须在 HTTPS 后启用 Secure Cookie。
- 身份验证实时读取 User、Credential、Membership、到期、撤销、停用、软删除和强制改密状态，治理变更立即生效。
- 登录退避使用 HMAC 键控且有上限；公共端点不能泄露邮箱是否存在。

`EnvironmentRoot` 由部署配置声明并在事务中核对。Root 密码通过宿主秘密轮换和重启修改，不能由读取现有凭据的 UI 修改。Root 治理权限也不能绕过 Lab/Project 数据授权。

## 授权

权限同时核对 actor、Lab role、Project role、资源关系、operation 和 revision。外部 Token 只能进一步收窄人的有效权限。关闭外部 API 时，应在工具执行前卸载或拒绝 bearer 入口。

项目成员只看显式分配的动物及相关科研记录；同笼、同 Lab 或共享附件库均不自动构成授权。

## AI Provider 秘密

- **Server**：Provider 秘密按用户/档案版本使用 AES-256-GCM 加密，并用身份绑定 AAD。Master Key 版本推进前必须先成功解密并重新加密全部既有密文。
- **Desktop**：API Key 保留在 OS keyring；SQLite 只保存版本化 opaque reference。
- Root、Editor、Viewer 都不能读取或使用其他用户的 Provider 配置。
- API Key、密码、Session、CSRF、Token、签名私钥和解密后的 Provider body 不得进入 Debug、Audit、普通日志、Snapshot 或 UI state。
- 存在加密行时，Server 拒绝生成替代 Master Key。

## AI 执行控制

- 不向模型暴露 raw SQL、任意 URL、任意文件系统、账号/权限变更、migration 或部署控制。
- 工具 schema 固定且有边界；执行时再次核对授权和作用域。
- Ask/Auto/Full 不能覆盖人的权限、项目作用域、科研签署、加强审批或动作级禁止项。
- 模型写入先成为可审阅草稿。动物转移/死亡、删除、批量导入、用户治理、科研签署、图片证据批准和技术日志删除始终由人完成。
- Provider/模型失败只返回稳定脱敏诊断，不回显凭据、body 或内部数据库错误。

## 上传、附件与私有 AI 资产

文件名只是普通 metadata，不能成为路径或响应 header。上传分别限制大小、扩展名、media、结构、像素/帧和解压预算；图片必须净化后才能进入模型。对象读取/删除前校验 object key 和 hash；路径穿越、符号链接等攻击 fail closed。

私有 AI 图片、来源、Prompt、候选和 Job 只对 owner 可见。人工建立并批准正式关系前，它们不进入项目作用域或业务 Snapshot。

## Audit、Provenance 与日志

正式业务写入保存 actor、source、revision、时间、Audit 和适用的 Provenance。Audit 不是秘密仓库：transport 证明、Key、私有对象路径和敏感 Provider payload 必须排除。

Server 技术访问日志与正式 Audit/Provenance 分离。保留策略同时受条数和天数约束，只有 Environment Root 能预览策略变更或手动清理；正式 Audit/Provenance 不参与技术日志清理。

## 数据库与升级安全

已发布 migration 文件和 checksum 不可变。Server/Desktop 普通启动只做兼容核对，不会静默迁移既有稳定数据库。Schema 变更、备份、实际隔离恢复、Candidate 验证、激活与 Write Lease 属于升级控制面。

数据库、附件、数据产物、配置、generation manifest、密钥材料与 AI 状态组成一个恢复集合。目标首次写入前可以原子恢复已验证 source generation；首次写入后禁止自动降级，只能前向修复或执行带操作者数据损失确认的显式恢复。

禁止通过清空生产数据、手改 migration SQL、更换 Master Key 或删除附件 volume 绕过升级失败。

## 网络与部署

- PostgreSQL 和内部服务端口保持私有。
- 应用默认只发布到 loopback，在可信反向代理或文档规定的 Cloudflare Tunnel 终止 TLS。
- 不把 Docker socket 挂入 `muriarc-server`，也不让长期应用进程承担升级/备份权限。
- 生产环境启用 Secure Cookie 和精确 trusted origin。
- Cloudflare 公网 Profile 默认关闭外部 REST/MCP；启用时必须同时限定 host 和 Cloudflare service token。

详见 [Server 部署](DEPLOYMENT_cn.md)和 [Cloudflare 公网 Profile](CLOUDFLARE_PUBLIC_PROFILE_cn.md)。

## 安全报告

禁止在公开 issue 中提交真实动物数据、凭据、含身份信息的日志、数据库/附件归档或更新签名材料。应使用维护者私有渠道，提供受影响版本和合成复现步骤，并在共享诊断前轮换任何可能暴露的秘密。
