# Cloudflare 公网 Profile

## 安全边界

该 Profile 只支持 Cloudflare Tunnel 到单一同源 Web 域名：

```text
Browser -> Cloudflare Edge -> cloudflared(host systemd) -> 127.0.0.1:8787 -> MuriArc
                                      PostgreSQL remains private
```

`cloudflared` 是独立低权限宿主机服务。`muriarc-server` 仍只监听 loopback；8787、PostgreSQL
和 Origin IP 不得配置公网入站规则。Tunnel credential JSON 只属于 `/etc/cloudflared`，不得进入
MuriArc 数据库、`/etc/muriarc` 备份、Candidate、Upgrade Journal、Fixture 或日志。

本 Profile 按产品决定**不启用 Cloudflare Access 和 MFA**。MuriArc 账号、Lab/Project role、
CSRF、Audit 和 Write Lease 仍是应用授权边界。密码认证仍有不可消除的钓鱼与终端失陷风险；
Cloudflare Edge 会终止 TLS，因此能够处理解密后的 HTTP 内容、附件和 AI 请求。若该风险不可接受，
部署者必须改用未来的 MFA/Access Profile，而不是把 Origin 直接暴露。

## Native/systemd

1. 用 `muriarcctl install --profile native-system` 安装已签名 bundle；保持
   `MURIARC_BIND_ADDR=127.0.0.1:8787` 和 secure Session cookie。
2. 安装已固定版本并由发行渠道验证的 `cloudflared`，再安装 bundle 内：
   `cloudflared.service`、`cloudflared.sysusers`、`cloudflared.tmpfiles`。
3. 从 `muriarc.yml.example` 创建命名 Tunnel 配置，credential JSON 权限为
   `root:cloudflared 0640`；DNS 只指向 Tunnel，不建立 Origin A/AAAA 记录。
4. 生成 32 随机字节并以单行 base64 保存为 `/etc/muriarc/secrets/auth-rate-limit-key`
   （`root:muriarc 0640`），安装 `muriarc-cloudflare-public.conf.example` drop-in。
5. 执行 `systemd-analyze verify`、`cloudflared tunnel ingress validate`、
   `muriarcctl doctor`，然后依次重启 MuriArc 和 Tunnel；只从外部网络验证 Web hostname。

Public Profile 缺少 HMAC key、使用不安全 Cookie 或附件/data generation 不一致时 Server
fail closed。该 HMAC key 属于恢复集合之外的部署秘密：恢复到另一台机器时生成新 key 只会清空
攻击退避身份，不改变账号、密码 hash 或业务数据。

## Managed Compose

`cloudflared` 仍运行在宿主机，不进入 Compose，也不挂载 Docker socket。通过：

```bash
docker compose \
  -f deploy/managed-compose/compose.yaml \
  -f deploy/cloudflare-public/compose.override.yaml \
  --env-file /srv/muriarc/config/server.env up -d
```

上面是源码树中的模板路径；签名 Managed Compose bundle 内的对应路径为
`deploy/compose.yaml` 与 `deploy/cloudflare/compose.override.yaml`。正式部署只能由宿主机
`muriarcctl` 使用 bundle 中已校验的文件和两个 env-file 执行，示例命令仅用于说明叠加关系，
不能替代控制器。

启用 95 MiB Public Profile 和登录退避。宿主机 key 文件通过 Docker secret 只读挂载；
`server_data`、PostgreSQL volume、Keyset 与 active generation 仍由 `muriarcctl` 统一备份和切换。
容器内 Server 为接受 Docker 端口转发而监听 `0.0.0.0:8787`，但签名 Compose 模板只允许宿主机
发布 `127.0.0.1:8787:8787`；不得改成全接口发布。禁止 Watchtower、`latest`、直接
`docker compose pull/up` 绕过控制器。

## 密码和登录退避

- credential policy revision 2 最少 15 个 Unicode 字符、最多 1024 bytes、禁止控制字符；不要求
  大小写/数字/符号组合，也不周期性强制修改。
- revision 1 的既有账号在下一次**正确**登录后获得一个只用于改密/退出的 Session，并设置
  `must_change_password=true`；业务 API 和 external token 在完成改密前继续阻断。
- Environment Root、新账号临时密码、管理员重置和自助改密都使用当前 revision。
- 登录失败以 HMAC(normalized identity) 保存，不保存探测邮箱；第 5 次起 30 秒退避，指数增长并
  封顶 15 分钟。未知账号和冷却期均执行 Argon2 dummy 路径，返回相同的 401/error body。
- Cloudflare 还必须对 Web hostname 配置登录路径限流和 Managed Challenge；它们是边缘补偿，
  不能取代 PostgreSQL 持久退避。

## Cloudflare WAF、限流和缓存

在 Cloudflare Dashboard 中使用版本化/可导出的规则，RC 必须保存规则截图或 API 导出证据：

1. 对 `POST /api/v1/auth/login` 配置每 IP/短窗口限流，超限 Managed Challenge；不要根据响应差异
   创建账号枚举侧信道。
2. 对异常扫描、已知攻击 payload 和高风险 ASN 使用 WAF Managed Rules/Managed Challenge；
   不要挑战已登录 API 的每个请求，否则可能破坏流式上传。
3. 下列路径始终 **Bypass cache**：`/api/*`、`/mcp`、认证、附件上传/下载、AI、导入/导出、
   health/compatibility。Server 也为动态响应发送 `Cache-Control: no-store, private`。
4. 只有带内容 hash 文件名的静态 UI 资产（例如 `/assets/app.<hash>.js`）可缓存 immutable；
   `index.html`、Service Worker 和无 hash 文件不得长期缓存。
5. 不创建修改 `CF-Connecting-IP`、`X-Forwarded-For` 或 Host 的 Transform Rule。当前 Server 不信任
   代理 IP 头；审计只使用应用身份和 request id。未来只有在受信 Tunnel 边界验证后才可采纳
   `CF-Connecting-IP`。

## 外部 REST/MCP（默认关闭）

Web UI 使用同源 Session API，不等于开放 external bearer API。生产 main 默认
`MURIARC_EXTERNAL_API_ENABLED=false`，此时 bearer 身份被拒绝且 `/mcp` 不挂载。

确需集成时必须新增**独立 API hostname**，在 Cloudflare Access 上只允许 Service Token，并安装
可选 external-api drop-in/Compose override。每个请求必须同时满足：

1. 精确 Host 匹配专用 API hostname；
2. `CF-Access-Client-Id` 和 `CF-Access-Client-Secret` 匹配部署时只读 credential；
3. `Authorization: Bearer mat_...` 是 MuriArc 内部可撤销、用户绑定、scope 收窄的 token。

缺任一层都返回拒绝。普通浏览器 Session 不能升级为 MCP；external token 创建和撤销还要求当前
密码 step-up。Tunnel credential 与 Access Service Token 是不同秘密，均不得进入 MuriArc 恢复集。
Native 将 Service Token credential 文件保存在 `/etc/cloudflared`；Compose 也只从该类宿主机
部署秘密路径挂载，不能把它们放进 `server.env` 或 `server_data`。

## 上传能力

`GET /api/v1/runtime/capabilities` 是公开、只读、无秘密的运行时契约。Remote UI 在路由初始化时读取
该契约并在附件传输前检查限制。Public Profile 的普通附件上限为 **95 MiB**；Server 同时检查
`Content-Length` 并以流式 writer 硬限制实际字节。普通 JSON 仍是 1 MiB，导入和 AI source 仍是
各自 32 MiB，不能因附件上限而放大。95 MiB 以上的分块、可恢复上传属于未来单独的 Gateway/
Persistence 兼容扩展。

## RC 门禁

源码测试不能替代真实 Cloudflare staging。1.0 RC 必须用最终 Native/Compose digest 验证：Origin
端口不可达、唯一 hostname 可用、WAF/限流/Challenge、缓存 bypass、94/95/95+ MiB 边界、错误
Service Token/Host/bearer、Tunnel 断开恢复、升级 Drain/只读窗口和旧数据继续写入。任何 FAIL/SKIP
阻断发布。
