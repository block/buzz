# Buzz 长期升级边界

## 目标

业务台以 Buzz 为协作、会话和 Agent 运行底座，但业务身份、授权、审计、查询和执行契约不成为 Buzz 核心实现的一部分。升级 Buzz 时，只适配少量稳定插槽，不在上游会话流程中重复合并业务规则。

## 组件边界

| 层 | 所有者 | 允许职责 | 禁止职责 |
|---|---|---|---|
| Buzz 核心 | 上游 Buzz | Relay、Nostr 事件、ACP 传输、通用会话、桌面壳层 | 业务权限、业务工具名、业务结果解析 |
| Buzz 扩展契约 | 本项目 | 回合观察器、回合 MCP 注入、桌面 Provider/顶部动作/Dock 插槽 | 具体 IAM 规则或业务数据结构 |
| Business Adapter | 业务台 | 独立/Agent 代理身份、临时授权、工具结果审计、答复发布 | 修改 ACP 协议语义 |
| Business IAM | 业务台 | 主体、角色、资源、动作、数据范围、委托、决策、审计 | Buzz 频道成员管理 |
| Authentik | 身份平台 | 人类登录、MFA、OIDC、Step-up | 业务级资源授权 |

## 当前稳定插槽

1. `crates/buzz-acp/src/turn_observer.rs`

   ACP 客户端仅把标准化的 `session/update` 交给可选观察器。`TurnExtension` 统一管理回合授权、临时 MCP、观察和收尾，`lib.rs`/`pool.rs` 不再识别具体业务扩展。
2. `crates/buzz-acp/src/product_extensions.rs`

   业务适配器的组装根。业务工具识别、结果计数和答复捕获只存在于该边界下的 `business_agent.rs` 和 `business_response.rs`。
3. `desktop/src/extensions/AppExtensionProviders.tsx`
4. `desktop/src/extensions/AppExtensionTopChromeActions.tsx`
5. `desktop/src/extensions/AppExtensionLayout.tsx`
6. `desktop/src/extensions/AppExtensionDock.tsx`

   Buzz 桌面核心只渲染通用扩展点，不直接引用 Business Dock 或 Workbench Auth。`AppExtensionLayout` 负责为固定 Dock 提供正确的横向布局，业务 Dock 自身仍完全位于 feature 目录。

## Agent 权限模型

### 独立 Agent

- 是 Business IAM 中的持久业务主体，与普通人员使用同一套角色、数据范围和职责分离规则。
- 拥有自己的身份和权限，不继承创建者的权限。
- 高风险写入仍可要求人工审批、双人复核或 Step-up。

### 代理 Agent

- 不持有业务系统的长期权限。
- 每次执行由 Business IAM 根据被代理人当前有效权限、请求动作、资源和数据范围计算最小权限交集。
- 签发绑定 `human + agent + task + resource + action + scope + trace` 的短时委托凭证。
- 任务结束立即撤销；TTL 只是异常情况下的上限，不是正常回收机制。
- 审计同时记录被代理人、Agent、决策快照和最终结果。

## 升级流程

1. 更新 `origin/main`，记录 Buzz 上游 commit 和发布 tag。
2. 运行 `scripts/check-business-extension-boundary.sh`，先阻止新的核心侵入。
3. 运行 `scripts/check-buzz-upgrade-compatibility.sh origin/main`。脚本使用隔离索引收集相对共同祖先的全部已提交、暂存、未暂存和未跟踪业务台文件，并在临时工作树中预演完整补丁。
4. 在独立升级分支中合并 Buzz；先解决扩展插槽，再处理业务适配器。
5. 运行 Buzz 原生单元测试、桌面类型检查，再运行 Business Adapter/IAM 契约测试。
6. 用真实工作流验证：人类登录、独立 Agent 查询，代理 Agent 临时查询/写入、任务结束后凭证失效。
7. 在升级记录中保存上游 commit、契约版本、数据迁移版本和验证证据。

## 当前升级基线（2026-08-24）

- 本地 Buzz 基线落后 `origin/main` 74 个提交。
- 检查目标为 `0720f5380ce8a6c050afac159f8462c06cd51ab5`；最新桌面发布为 `desktop-v0.5.18`。
- 完整业务台补丁（40 个上游跟踪文件和 451 个业务台新增文件）已迁移到独立分支 `codex/business-platform-buzz-latest`。三方合并只有 5 个需要人工适配的文件：`crates/buzz-acp/src/lib.rs`、`desktop/src-tauri/src/lib.rs`、`desktop/src/app/AppShell.tsx`、`desktop/src/shared/hooks/useThreadPanelWidth.ts` 和 `pnpm-lock.yaml`。
- 适配后的完整候选已通过 Business IAM/gateway/query/action/core Rust 编译、Desktop TypeScript 检查、Desktop E2E 构建、Tauri 检查、Business Web 构建及 18 项测试、Desktop 5475 项测试，以及 Business Dock 11 项 E2E。详细证据见 [`upgrade-record-2026-08-24.md`](upgrade-record-2026-08-24.md)。
- 业务 IAM、Authentik、gateway 和 Business Dock feature 本身不进入 Buzz 上游核心合并；它们通过扩展组装点重新接线。

## 尚需继续收敛的点

- `AppShell.tsx` 已只依赖一个通用 `AppExtensionLayout`，语义耦合已收敛；但 JSX 包装使该文件相对上游仍有较大的纯缩进差异，升级时可能出现机械合并冲突，继续把它列为固定适配点并用 E2E 验证。
- Business IAM 已有独立策略 crate、独立 schema、gateway 适配器和离线管理工具，但尚未作为独立生产权限服务部署。
- Agent Host 当前只开放固定读能力目录；读委托已经由 Business IAM 计算独立 Agent 自身权限或代理 Agent 的按任务最小权限交集。写能力仍保持 `V7_BLOCKED`。
