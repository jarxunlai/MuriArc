# MuriArc Server 正式交付与安全升级边界

本文描述 `1.0 / E0001` 起的正式 Server 交付契约。根目录的开发
`docker-compose.yml` 仍用于源码构建和 preview 验证；正式安装必须使用签名
Server bundle 中的 `deploy/native-system` 或 `deploy/managed-compose`，二者不能混用。

## 1. 权限与数据边界

长期运行的 `muriarc-server` 只以低权限 `muriarc` 身份访问当前 generation：

- 它不拥有 systemd 控制权、Docker socket、数据库 DDL 或备份权限；
- Native 只写 `/var/lib/muriarc`，读取 `/opt/muriarc/current` 与
  `/etc/muriarc/server.env`；
- Managed Compose 的应用容器只挂 `server_data`，不挂 Docker socket；
- `muriarc-upgrade-executor` 是一次性控制进程，只对隔离 Candidate 执行 DDL；
- `muriarcctl` 在宿主机执行安装、备份、恢复、切换与服务控制。

一个可恢复 generation 必须同时包含 PostgreSQL、data/attachments、配置、Keyset、AI
状态和 `deployment-generation.json`。只恢复数据库、只恢复 volume，或让 Server 自动创建
空目录/替代 Master Key，都不是成功恢复。

## 2. 签名 bundle

构建脚本只接受已生成的最终制品，不会调用编译器或签名器：

```bash
python3 scripts/build_server_bundle.py \
  --profile native-system \
  --version 1.0.0 \
  --output /absolute/path/outside-git/muriarc-native-1.0.0 \
  --server /absolute/path/muriarc-server \
  --controller /absolute/path/muriarcctl \
  --upgrade-executor /absolute/path/muriarc-upgrade-executor \
  --verifier /absolute/path/muriarc-verifier \
  --ui-dir /absolute/path/ui-dist
```

脚本拒绝 symlink、空文件、路径逃逸、已存在输出目录和 Git 工作树内输出。生成结果包含
闭合的 `bundle-manifest.json`。发布流水线必须把脚本输出的
`manifest_object_digest` 写入经过 TUF 角色签名的目标元数据；安装时由可信的 bootstrap
流程提供该 digest：

```bash
export MURIARCCTL_BUNDLE_ROOT=/absolute/path/extracted-bundle
export MURIARCCTL_TRUSTED_BUNDLE_MANIFEST_DIGEST=sha256:<64-hex>
```

仅有下载链接、tag、TLS 或文件名都不能替代签名元数据。

Release Manifest 是 TUF target custom metadata/正式 RC 的外部签名对象，**不嵌入它所描述的
Native 或 Compose bundle**。否则 Manifest 内的 profile artifact digest 会依赖包含自身的 bundle，
形成不可解的自引用。发布流水线先封装最终 bundle/镜像/Windows 安装包并取得 digest，再生成外部
Release Manifest；控制器从已验证元数据把 Manifest 交给 Upgrade Engine/Executor。bundle 内只含
自己的闭合 `bundle-manifest.json`，安装时同时验证外层 target digest 和内层闭合清单 digest。

## 3. Native/systemd

Native 使用固定系统布局：

```text
/opt/muriarc/releases/<version>/  不可变版本目录
/opt/muriarc/current              原子 release symlink
/etc/muriarc/server.env           root:muriarc 0640，身份与运行配置
/var/lib/muriarc/control/active.env  root:root 0600，当前 generation 指针
/var/lib/muriarc/generations/<uuid>/  database 对应的 data/attachments/keyset
/var/lib/muriarc/backups/         已验证恢复点
/var/lib/muriarc/candidates/      临时 Candidate 控制数据
```

安装命令必须以 root 运行：

```bash
sudo --preserve-env=MURIARCCTL_BUNDLE_ROOT,MURIARCCTL_TRUSTED_BUNDLE_MANIFEST_DIGEST \
  ./muriarcctl install --profile native-system
```

安装会验证闭合清单、stage 不可变 release、更新 `current`，安装 systemd/sysusers/tmpfiles
模板并 `enable` unit，但**不会自动启动**。管理员必须根据
`/etc/muriarc/server.env.example` 和
`/var/lib/muriarc/control/active.env.example` 建立两个真实 root-only 文件，创建同一
generation 的目录、数据库状态和 manifest，再运行：

```bash
sudo muriarcctl doctor --output json
sudo systemctl start muriarc.service
curl --fail http://127.0.0.1:8787/livez
curl --fail http://127.0.0.1:8787/readyz
```

`livez` 仅表示进程存活；`readyz` 还要求精确 Epoch/Digest/Generation、有效 Write Lease
（只读激活除外）、真实 data/attachment 根、可用 AI Master Key 和 UI `index.html`。

## 4. Managed Compose

Managed Compose 不是“任意 compose 文件 + Watchtower”。安装根必须是绝对路径：

```bash
export MURIARCCTL_INSTALL_ROOT=/srv/muriarc
./muriarcctl install --profile managed-compose
```

复制并填充：

```text
/srv/muriarc/config/server.env.example -> server.env
/srv/muriarc/control/active.env.example -> active.env
```

`server.env` 保存 digest-pinned GHCR 镜像与身份/密码配置；`active.env` 只保存当前
PostgreSQL database、generation UUID 和 `read-write|read-only` 激活模式。宿主机控制器固定
使用两个 `--env-file` 调用 `/usr/bin/docker compose`。不要直接 `docker compose pull/up`
绕过控制器，不要加入 `build:`、`latest`、Watchtower 或 Docker socket mount。

PostgreSQL 不发布 host port，Server 只发布 `127.0.0.1:8787`。不同 generation 在
`server_data` volume 内使用互不重叠目录；Candidate 使用独立 PostgreSQL database 和目录，
且外部 Provider、后台 Job 与真实用户写入均被禁止。

## 5. 升级与维护窗口

正式控制面顺序固定为：签名目标验证、三重锁、预检、graceful drain、冻结 Write Lease、
联合备份、实际隔离恢复、Candidate migration、七层验证、原子 activation pointer 切换、
只读启动验证、开放新 Write Lease。普通 Server 启动不执行 DDL。

单节点不承诺零停机：M0/UI 通常只有短切换，M1 会短时冻结写入，M2 可提供只读窗口，M3
明确离线。只读激活不是普通业务模式：认证 Session touch 也属于写入；对外流量必须保持
阻断，只允许控制器访问健康端点和无副作用 verifier 路径。

新版首次写入前，失败可以把 activation pointer 切回旧 generation 并恢复旧 Write Lease；
一旦 `first_write_at` 出现，自动降级被永久拒绝，只能 forward-fix，或由管理员显式恢复并确认
可能丢失新版写入。

当前仓库仍是 `0.1.0 / preview_epoch_0`。`muriarcctl upgrade` 在最终 Native/Compose 物理
Driver 和正式签名制品没有通过 1.0 RC 前会 fail closed；不会把模板/contract test 冒充为
已经可用于真实数据的升级成功。

## 6. BYO 与恢复点

BYO PostgreSQL/存储只有在能证明以下能力时才可升级：PostgreSQL 17、隔离 Candidate database、
完整 dump/restore、generation 目录复制、DDL executor、七层 verifier 和服务控制。任一能力缺失，
`doctor`/`upgrade` 都必须失败，不能回退到在线原地 migration。

每次升级产生的最后一个已验证恢复点默认不可删除；仅
`muriarcctl recovery prune --backup <uuid>` 可清理更早的明确对象。数据库 dump、附件、Key、
Journal 和验证报告全部保存在 Git 之外。

## 7. Cloudflare 公网 Profile

公网部署不得直接开放 Origin。Native 和 Managed Compose bundle 都携带独立宿主机
`cloudflared` 模板与 Public Profile override；完整安装、安全补偿和 RC 边界见
[CLOUDFLARE_PUBLIC_PROFILE.md](CLOUDFLARE_PUBLIC_PROFILE.md)。

## 8. 最终 1.0 RC 编排

正式发布不能只运行某一种部署 smoke。`scripts/run-release-candidate.sh` 把完整历史 Fixture 七层
矩阵、Native/systemd、Managed Compose、Windows 安装包、Cloudflare staging、恢复/故障注入、
首次写入降级保护和签名攻击证据绑定到最终 Release Manifest。`release-fixtures/rc-gate.json`
是不可弱化的 required-scenario definition；`scripts/check_release_readiness.py` 只在所有记录为
`pass`、`final_package`、`fail_count=0`、`skip_count=0`，且外部签名 `artifact-lock.json`、
artifact/provenance/signature evidence/digest 完全一致时生成仓库外
`release-readiness-report.json`。

当前 preview 源码、空 Catalog 或缺少真实 RC Driver 都会失败。这表示发布控制面已 fail closed，
不表示 1.0 已发布或真实用户数据已经通过验收。完整命令和证据格式见
[RELEASE_EVIDENCE.md](RELEASE_EVIDENCE.md)。
