# Cloudflare 公网 Profile

> [English](CLOUDFLARE_PUBLIC_PROFILE.md) | 简体中文

## 状态与范围

该可选 Profile 通过 Cloudflare Tunnel 暴露一个 MuriArc Server Origin，面向签名 Native/systemd 或 Managed Compose 交付；它不代表生产公网托管或 `1.0.0 / E0001` RC 已通过。

```text
Browser -> Cloudflare Edge -> cloudflared（宿主服务） -> 127.0.0.1:8787 -> MuriArc
                                                          PostgreSQL 保持私有
```

`cloudflared` 是独立低权限宿主进程；`muriarc-server` 仍绑定 loopback。8787、PostgreSQL、Docker socket、control file 和 Candidate endpoint 都不能直接公网暴露。

## 安全边界

没有强制 Cloudflare Access MFA 的浏览器科研应用仍有凭据钓鱼和账号接管风险。该 Profile 要求以下补偿控制，但不宣称它们等价于 MFA：

- 长密码与 Argon2id；
- HMAC-keyed 登录退避与通用响应；
- Secure/HttpOnly/SameSite Cookie 与 CSRF；
- 精确 host/origin；
- Cloudflare managed rules、WAF/限流、bot/滥用监控，以及认证响应不缓存；
- 账号变化/停用后立即撤销 Session/Token；
- 独立 Environment Root 治理与受保护宿主配置；
- 定期恢复演练和安全日志复核。

若部署所有者需要 MFA，必须另行设计和测试上游 identity/access；禁止把此 no-MFA Profile 描述为提供 MFA。

## Native/systemd

从已验证 Server bundle 安装 Profile 的 `cloudflared` service/template。Tunnel token/credential 只作为 root-owned 宿主秘密，不进入 MuriArc 环境、数据库、bundle、Git 或应用日志。

Service 只把一个精确公网 hostname 路由到 loopback，不使用宽泛 wildcard ingress、直接 Origin DNS 或指向 PostgreSQL/metrics/control 的路由。先确认 MuriArc readiness，再开放 Tunnel 流量。

## Managed Compose

`cloudflared` 仍是宿主服务，不成为带 Docker socket 的容器。Managed Compose 只发布 `127.0.0.1:8787`，禁止增加公网 port、`network_mode: host`、Watchtower、浮动 image tag 或 socket mount。

## 登录与密码控制

Cloudflare 限流只补充 Server HMAC-keyed 登录退避，不能替代。限流 key/响应不能泄露邮箱是否存在。Root credential 仍由环境管理，不通过公网端点 reset。

监控重复认证失败、Token 滥用、异常国家/ASN、WAF 和应用安全事件，但不记录密码、Cookie、CSRF、Token 或 Provider Key。

## WAF、限制与缓存

- 只缓存带 fingerprint 的静态 UI 资产；
- API、认证、MCP、下载、私有图片、附件和包含部署状态的 health 不缓存；
- 保留受控 AI 请求需要的 stream/timeout；
- 对登录、改密、上传、AI turn、Token 创建和下载设置 method/path-aware 限流；
- 拒绝模糊 host/proxy header 和直接 Origin。

Edge 上传 envelope 上限为 **95 MiB**，让 Cloudflare 限制可预测失败；应用各端点继续执行更严格独立限制。Edge 允许值不提高应用限制。

## 外部 REST/MCP

外部 bearer REST/MCP 默认关闭。显式启用时，同时要求精确公网 host、Cloudflare service-token header 与实时 MuriArc 用户 scope Token。Cloudflare credential 不授予 MuriArc 权限；MuriArc Token 也不能绕过 edge policy。

浏览器 MCP origin 使用精确 allowlist；非浏览器通常不带 `Origin`，但仍需要同一实时 scope 授权。

## 数据与 Provider 流量

Cloudflare 终止公网 TLS，但 PostgreSQL、备份、Key material 和 Provider credential 保持在宿主/应用私有边界。Provider API 由 MuriArc 请求用户配置 endpoint，不经公网 Browser route 中转。

私有图片/附件继续执行 owner/project 授权；猜中 URL、缓存响应或 Cloudflare authenticated identity 都不够。

## RC 门禁

Cloudflare staging 是正式 RC 的必需物理场景。证据必须使用最终签名 bundle/image digest，并覆盖：

- 精确 hostname/Tunnel/Origin 拓扑；
- 直接 Origin 拒绝；
- HTTPS、Cookie、CSRF、登录退避、WAF/限流、cache；
- 95 MiB edge 与更严格应用限制；
- 外部 API 默认关闭，启用时双控制；
- 恢复、重启、配置轮换和日志脱敏；
- FAIL=0、SKIP=0。

合同测试或本机 `cloudflared` 模板不能单独构成 RC PASS。
