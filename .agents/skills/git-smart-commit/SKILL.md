---
name: Git Smart Commit
description: 将杂乱的 git 变更，按功能逻辑自动拆分成多个有意义的 conventional commit
---

# Git Smart Commit — 智能拆分提交

将目前所有 staged / unstaged 变更，按功能逻辑分群后，逐批 `git add` + `git commit`。

---

## 流程

### 1. 检查变更状态

执行以下命令获取完整变更清单：

```bash
git status --short
```

若没有任何变更，告知用户「目前没有需要提交的变更」后结束。

接着获取所有变更的 diff 内容（用来判断分群逻辑）：

```bash
git diff
git diff --cached
```

---

### 2. 分析并分群

根据以下维度，将文件变更分成多个 **commit 群组**，每组代表一个独立的逻辑单元：

#### 分群依据（优先顺序）

| 优先级 | 维度 | 示例 |
|--------|------|------|
| 1 | **项目脚手架 / 配置文件** | `package.json`, `vite.config.*`, `.gitignore`, `README.md`, `tsconfig.json` |
| 2 | **数据层 / config data** | `src/data/*.js`, `src/constants/*`, `src/config/*` |
| 3 | **组件（按组件名称分组）** | `src/components/Hero.jsx` + 对应测试 + 对应样式 |
| 4 | **页面 / 路由** | `src/pages/*`, `src/routes/*`, `src/App.jsx` |
| 5 | **全局样式** | `src/index.css`, `src/styles/*`, `src/theme/*` |
| 6 | **工具 / hooks / 类型** | `src/utils/*`, `src/hooks/*`, `src/types/*` |
| 7 | **测试** | `__tests__/*`, `*.test.*`, `*.spec.*` |
| 8 | **文档 / 其他** | `docs/*`, `*.md`（非 README）, 其他杂项 |

#### 分群规则

- **同一组件的 JSX/TSX + CSS Module + 测试 → 归为同一组**
- **相关的数据文件如果是为某个组件服务 → 可考虑合并或独立**，取决于变更量
- **若某一组只有 1 个文件且改动极小（< 5 行）→ 合并到最相关的邻近组**
- **新增文件用 `feat`，修改用 `fix` / `refactor` / `style`，删除用 `chore`**

---

### 3. 产出 Commit 计划

在执行任何 git 操作之前，先列出计划让用户确认：

```
📋 Commit 计划（共 N 个 commit）

1. chore(project): 初始化项目配置与依赖包
   → package.json, vite.config.js, .gitignore

2. feat(data): 新增首页各区块的配置数据
   → src/data/navigation.js, src/data/hero.js, ...

3. feat(navbar): 新增 Navbar 组件（含 RWD 汉堡菜单）
   → src/components/Navbar.jsx

...

确认执行？(Y/n)
```

使用 `notify_user` 工具向用户展示计划并等待确认。

---

### 4. 逐批执行 Commit

用户确认后，对每一组依次执行：

```bash
git add <file1> <file2> ...
git commit -m "<type>(<scope>): <subject>"
```

#### Commit Message 格式

```
<type>(<scope>): <简短描述，简体中文>
```

**type 对照表：**

| type | 使用时机 |
|------|---------|
| `feat` | 新增功能、组件、页面 |
| `fix` | 修复 bug |
| `style` | 纯样式调整（不影响逻辑） |
| `refactor` | 重构（不改变行为） |
| `chore` | 杂务（配置文件、脚手架、CI） |
| `docs` | 文档更新 |
| `test` | 测试相关 |

**scope 规则：**
- 组件：用组件名称小写，例如 `hero`, `navbar`, `pricing`
- 数据层：`data`
- 全局样式：`style`
- 项目配置：`project`
- 多个范围：用最主要的一个，不要用斜线串接

**subject 规则：**
- 使用简体中文
- 不超过 50 字
- 不以句号结尾
- 用「动词开头」：新增、调整、修正、移除、重构

---

### 5. 确认结果

所有 commit 完成后，执行：

```bash
git log --oneline -20
```

将结果展示给用户，确认所有 commit 都已正确建立。

---

## 边界情况处理

- **有冲突或 merge 状态**：提醒用户先解决冲突，不执行任何操作
- **有 `.env` 或敏感文件**：提醒用户确认是否应被 gitignore，不自动提交
- **变更量极大（> 50 个文件）**：先产出分组摘要，请用户确认后再执行
- **用户已有部分 staged 变更**：尊重已 staged 的状态，将其视为一个独立群组或合并到最相关的群组
