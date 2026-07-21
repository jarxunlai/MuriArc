# ADR-0008：双运行形态与账号安全边界

- 状态：Accepted
- 日期：2026-07-21

## 背景

MuriArc 同时面向个人本地 Desktop 和共享实验室 Server。若拆成两个代码项目，领域模型、Store
contract、业务 UI 和迁移规则会快速分叉；若强行共用同一认证模型，又会在单机 SQLite 中引入没有
实际安全边界的密码表。同时，Server 不能继续依赖写死或一次性 bootstrap 的管理员密码，且
LabAdmin 之上需要一个由部署所有者控制的恢复与治理身份。

## 决策

### 单代码库、两个安全边界

- Desktop/Tauri/SQLite 与 Server/Axum/PostgreSQL 共享 Domain、Application、Store contract 和主体 Vue UI。
- 仅认证、transport、数据库、密钥和部署适配层按运行形态拆分，不建立两个产品仓库。
- Desktop 不创建账号、Credential、Session 或 token 表。每次 WebView 会话先显示无密码“进入本地空间”，
  用 `sessionStorage` 避免刷新重复；该步骤只确认 Lab/LocalOperator，明确不是安全锁。
- Server 使用持久 Argon2id Credential、HttpOnly Session、CSRF、可撤销 external token 和实时角色检查。

### Environment Root

- Server 必须从环境读取 Lab 名称、Root User ID、邮箱、名称和密码。
- Root 密码按产品所有者确认保存在宿主机 `.env`，数据库只保存 Argon2id hash；真实 `.env` 不进入 Git，
  文件权限要求 600，并接受宿主机管理员、Docker inspect 和备份仍可暴露明文这一部署风险。
- 每次启动在单事务中创建或核对 Lab、Root User、LabAdmin membership 和 Credential。环境密码变化时
  更新 hash/revision 并撤销 Root Session；冲突和软删除阻止启动，所有 Audit 脱敏。
- Root 是配置声明的唯一 User ID 加 LabAdmin 权限，不新增平行的领域角色枚举。Root 本身不能在应用内
  修改、停用、降级或重置；修改 `.env` 并重启是唯一 Root 凭据变更路径。

### 治理层级和密码生命周期

1. Environment Root 能治理全部应用账号，并独占 LabAdmin 的创建、修改、停用、降级与重置。
2. LabAdmin 治理全部非 LabAdmin 账号和实验室业务，不能治理 Root、同级管理员、环境或代码。
3. ProjectAdmin 只在获授权 Project 内管理项目。
4. AnimalManager、Editor、Viewer 分别提供 Lab Registry、项目写入和只读权限。

新账号使用临时密码并强制首次改密。密码只要求至少 8 个 Unicode 字符、最多 1024 UTF-8 bytes、
无控制字符且新旧不同；强度等级只作建议。强制期只开放 Session/CSRF、退出和自助改密，其他业务
与 bearer 能力返回 `password_change_required`。自助改密撤销其他 Session；管理员重置撤销目标全部
Session 和 external token，并再次强制改密。管理员和响应永远不能查看已有密码或 hash。

## 后果

- 领域和 UI 可以持续共享，Desktop 不承担伪安全认证复杂度，Server 仍有明确的多用户安全边界。
- Root 明文环境密码成为必须纳入宿主机、Docker 与备份威胁模型的部署秘密；权限 600 不是对 root/daemon 的保护。
- 旧 bootstrap seed 部署必须显式迁移 Root 环境变量，不能静默沿用遗留密码。
- Root 恢复依赖部署控制权而非应用内“查看密码”或万能重置接口。
- Credential lifecycle 只存在 PostgreSQL；SQLite migration 和 Desktop Store contract 不新增认证表。

## 非目标

- 不把 Desktop 欢迎页描述为系统登录或磁盘加密。
- 不实现跨 Lab Root、多个 Environment Root 或应用内 `.env` 编辑器。
- 不强制复杂字符组合、定期过期或管理员可读密码。
- 不实现 Desktop 与 Server 实时同步。
