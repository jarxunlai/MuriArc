# Feature Spec: Cloudflare 公网安全 Profile

> 本分支只实现公网暴露的补偿控制与部署模板；不改变 Upgrade Engine、Candidate generation 或正式发布签名规则。

## 分支信息

| 项目 | 值 |
|---|---|
| 分支名称 | `feature/cloudflare-public-profile` |
| 基于提交 | `feature/desktop-safe-updater@5d32501` |
| Worktree 路径 | `/home/ljx/Github/animal_lab-cloudflare-public-profile` |
| 建立日期 | `2026-07-27` |

## 目标

以独立宿主机 `cloudflared` 把唯一 Web 同源域名转发到 `127.0.0.1:8787`，不公开 Origin、
PostgreSQL 或应用端口。按产品决定不启用 Cloudflare Access/MFA，因此必须由 MuriArc 自身实施
更强的密码策略、持久登录退避、统一错误、敏感操作密码 step-up、默认关闭的外部 bearer/MCP
边界，以及由 Server 运行时声明的 95 MiB 附件上限。

## 实现范围

- [x] 新增 credential policy revision；公网 Profile 为 revision 2、最少 15 个非控制字符，
  不要求字符组合、周期改密。
- [x] 既有低 revision 凭据在下一次正确登录时标记 `must_change_password`；新密码、临时密码和
  Environment Root 使用当前 policy revision。
- [x] 使用带服务器秘密的 HMAC 身份摘要持久化登录失败和退避；未知账号仍走 dummy Argon2，
  冷却期不暴露账号存在性。
- [x] external bearer REST/MCP 默认关闭；启用时必须同时匹配独立 API hostname、
  Cloudflare Service Token headers 和 MuriArc bearer token。
- [x] external token 创建与撤销要求当前密码 step-up；管理员账号和权限操作继续沿用既有 step-up。
- [x] 增加公开只读 runtime capability endpoint；公网 Profile 的附件流式限制为 95 MiB，
  RemoteHttpGateway 在上传前读取并执行该限制，普通 JSON 仍为 1 MiB。
- [x] 提供独立 `cloudflared` systemd 模板、最小权限环境文件示例和部署检查脚本；Token 不写入
  MuriArc 数据库、配置备份、Upgrade Journal 或恢复集 manifest。
- [x] 文档固定 WAF/登录限流/Managed Challenge、缓存绕过、Origin 隐藏、不信任代理头、
  无 MFA 抗钓鱼剩余风险和 Cloudflare Edge 可见明文 HTTP 内容的边界。
- [x] 增加 migration checksum、Rust/前端/部署 policy tests。

## 非目标

- 不把 Docker socket 或 Cloudflare Token 挂入 `muriarc-server`。
- 不在本分支实现分块续传；95 MiB 以上附件是后续独立兼容扩展。
- 不增加远程升级 Web API，不允许 Cloudflare 路径绕过 Write Lease、权限、Audit 或 AI 审批。
- 不声称 Cloudflare staging、真实 WAF 或公网渗透测试已完成；最终 RC 必须使用真实域名和最终制品。

## 验收标准

- Public Profile 缺少 rate-limit HMAC key、API 专用 hostname/Service Token 成对配置或安全 cookie
  时启动 fail closed。
- 登录错误对不存在账号、错误密码和冷却状态保持同一 HTTP 状态/错误正文；数据库不保存原始探测邮箱。
- 正确旧密码可以建立仅用于强制改密的 Session，但不能继续访问业务 API 或 external token。
- 默认 bearer 与 `/mcp` 均不可用；开启后缺任一 Cloudflare header、错误 Host 或错误 MuriArc token
  都失败，浏览器 Session 不可隐式升级为 external identity。
- 95 MiB 可上传，超过 capability 的请求在 UI 和 Server 两端阻断；JSON、导入和 AI source 的独立
  小上限不被放大。
- `/api/*`、`/mcp`、认证、AI、附件和导出响应不可被共享缓存；只有内容哈希静态资产允许缓存。

## 跨分支备注

依赖 `5d32501` 及此前所有兼容、Upgrade、Fixture、Native/Compose 与 Desktop 阶段。最终
`feature/release-integration-1-0` 只能把本分支作为实现输入；真实 Cloudflare staging、E0001 fixture
和签名 RC 证据仍必须由最终制品生成，任何 `FAIL/SKIP` 阻断发布。
