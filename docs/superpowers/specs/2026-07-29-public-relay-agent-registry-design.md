# Buzz v0.5.0 公共 Relay Agent 注册与运行架构设计

日期：2026-07-29

## 1. 背景与问题

当前本地 PoC 中有五个通过 `buzz-acp` 接入 Relay 的公共基础 Agent。它们需要同时满足两类客户端：

- 局域网成员使用官方 Buzz Desktop v0.5.0 时，可以在指定 Channel 中 Mention 并与 Agent 交互。
- 本机使用定制 Buzz Desktop v0.5.0 时，可以在 Agents 模块中看到这些 Agent 的身份和在线状态。

Relay 侧若把成员角色设为 `bot`，本机定制版容易识别，但官方 v0.5.0 的 Mention 候选不完整；若设为 `member`，官方客户端可以 Mention，但当前本机定制版无法把它们识别为 Relay Agent。Relay 中又没有这五个 Agent 的 kind `10100` 声明事件，因此仅依赖 Relay 事件或 Channel 角色都无法同时满足两类客户端。

本设计增加一份由部署工具维护的本地公共 Agent 注册表。Relay 继续使用兼容性最好的 `member` 角色，定制 Desktop 通过注册表补充 Agent 身份判断。注册新 Agent 时，由同一条命令完成身份生成、显式 Channel 授权、Runner 配置、注册表同步和启动验证。

## 2. 目标

1. 保持 Buzz Desktop、Relay 和 `buzz-acp` 在 v0.5.0，不回退版本。
2. 现有五个 Agent 保持原公钥、私钥、工作目录、Channel 成员关系和运行行为。
3. 官方 v0.5.0 客户端仍可在指定 Channel 中 Mention 并与公共 Agent 交互。
4. 本机定制 Desktop 的 Agents 模块能显示已注册的公共 Relay Agent。
5. 新增公共 Agent 无需重新打包 Desktop。
6. 新 Agent 必须显式指定一个或多个 Channel，只加入指定 Channel。
7. 默认完成身份生成、Claude Code ACP 配置、隔离工作目录创建和 Runner 启动。
8. 注册表、日志和 Desktop 数据中不保存身份私钥或 Provider API Key。

## 3. 非目标

- 不把 Relay 改造成 Agent Runtime 或模型 Provider。
- 不让公共 Agent 自动加入全部现有或未来 Channel。
- 不向 Agent 授予主干合并、生产发布、Docker Socket 或敏感目录访问能力。
- 不要求局域网其他成员安装定制 Desktop；官方客户端只保证 Channel 内 Mention 与交互能力。
- 本阶段不引入企业身份、Secret Manager、高可用或跨主机 Runner 调度。

## 4. 核心决策

### 4.1 双层注册表

部署侧以以下文件作为唯一事实源：

`platform/agents/public-agents.json`

Desktop 使用以下运行时镜像：

`~/Library/Application Support/xyz.block.buzz.app/agents/public-relay-agents.json`

部署注册表包含启动 Runner 所需的非敏感元数据；Desktop 镜像只保留展示和身份匹配字段。更新过程先校验完整内容，再通过临时文件和原子重命名替换目标文件，避免 Desktop 读取到半写入 JSON。

### 4.2 Relay 成员角色保持 `member`

所有公共 Agent 在指定 Channel 中都使用 `member` 角色。注册表只帮助本机定制 Desktop 识别 Agent，不改变 Relay 权限模型，也不发布伪造的 kind `10100` 事件。

### 4.3 显式 Channel 授权

创建命令必须至少出现一次 `--channel`。命令不会读取“默认 Channel”，不会自动扩散到其他 Channel，也不会在未来新建 Channel 时自动加入。

### 4.4 一条命令完成注册和启动

对外入口为：

```text
platform/bin/create-public-agent \
  --id research \
  --name "Research Agent" \
  --channel <channel-uuid> \
  [--channel <channel-uuid>] \
  [--model <model>] \
  [--workdir <absolute-or-poc-relative-path>] \
  [--system-prompt-file <path>]
```

默认值：

- Provider：本机已配置的 Claude Code 自定义 Provider。
- Model：当前公共 Agent 使用的默认模型。
- `respond_to`：`anyone`。
- 私信边界：仅允许部署 Owner，其他私信拒绝。
- 工作目录：`deployment/buzz-local-poc/runner-workspaces/public/<agent-id>`。
- System Prompt：由命令生成包含公共协作边界的最小模板。

命令不得复制、输出或写入 Provider API Key；Runner 继承现有的安全凭据解析方式。

## 5. 数据模型

### 5.1 部署注册表

```json
{
  "version": 1,
  "agents": [
    {
      "id": "product",
      "name": "Product Agent",
      "pubkey": "<64-char-hex-pubkey>",
      "channelIds": [
        "<channel-uuid>"
      ],
      "configPath": "platform/agents/product/agent.env",
      "workdir": "worktrees/product",
      "state": "active",
      "enabled": true,
      "source": "builtin"
    }
  ]
}
```

字段约束：

- `id`：稳定、唯一、可用于文件名和 tmux window 名的 slug。
- `name`：Desktop 和 Channel 中显示的名称。
- `pubkey`：Nostr 十六进制公钥，不保存私钥。
- `channelIds`：非空、去重后的显式 Channel UUID 列表。
- `configPath`：PoC 根目录内的 Runner 非敏感配置路径。
- `workdir`：隔离工作目录。
- `state`：`provisioning`、`active` 或 `failed`。
- `enabled`：动态 supervisor 是否应启动该 Agent。
- `source`：`builtin` 表示迁移的现有五个 Agent，`public` 表示后续创建。

### 5.2 Desktop 镜像

```json
{
  "version": 1,
  "agents": [
    {
      "id": "product",
      "name": "Product Agent",
      "pubkey": "<64-char-hex-pubkey>",
      "channelIds": [
        "<channel-uuid>"
      ],
      "state": "active",
      "enabled": true
    }
  ]
}
```

Desktop 镜像不包含私钥、凭据、工作目录、Runner 配置路径或 System Prompt 路径。

身份私钥继续独立保存在 `platform/env/identities/<agent-id>.env`，权限为 `0600`。

## 6. Desktop 读取与合并

Tauri 后端增加只读命令，从应用数据目录读取 `agents/public-relay-agents.json`：

- 文件不存在时返回空列表。
- JSON 格式、版本或字段无效时返回可诊断错误，不影响本地 Managed Agents。
- 不允许前端传入任意文件路径。

Agents 模块将三类数据合并：

1. 本地 Managed Agents；
2. Relay kind `10100` Agent；
3. 当前 Channel 成员中，公钥存在于本地注册表且注册表包含该 Channel 的公共 Agent。

合并以公钥为稳定键，Relay kind `10100` 数据优先于本地注册表展示字段，避免重复卡片。本地注册表条目仅在以下条件全部满足时进入当前 Channel 的 Agent 列表：

- `enabled` 为 `true`；
- `state` 为 `active` 或 `failed`；
- 注册表包含当前 Channel；
- Relay 当前成员列表也包含该公钥。

`active` 条目按实时连接信息显示在线或离线；`failed` 条目始终显示为离线并保留诊断入口。普通 `member` 若不在注册表中，仍按人员处理。

页面进入、窗口重新聚焦及现有 Agent/Channel 刷新动作会重新读取注册表，因此新增 Agent 后无需重新构建或重启 Desktop。

## 7. 创建流程

`create-public-agent` 按以下顺序执行：

1. 解析参数，拒绝缺失 `--id`、`--name` 或 `--channel`。
2. 校验 `id` 格式和唯一性；若已存在则进入幂等检查，不生成第二份身份。
3. 向 Relay 校验所有 Channel 均存在，并确认当前部署 Owner 有添加成员的权限。
4. 校验可选工作目录和 System Prompt 文件；拒绝敏感目录、越界路径及不可读文件。
5. 生成 Nostr 身份，把私钥写入权限为 `0600` 的独立 identity 文件。
6. 创建隔离工作目录、Agent 配置和 System Prompt；配置 Claude Code ACP、模型、`respond_to=anyone` 及私信 Owner 边界。
7. 在部署注册表中写入 `provisioning` 条目，并同步 Desktop 镜像。
8. 依次将 Agent 加入每个显式 Channel，角色固定为 `member`。
9. 通过动态 supervisor 启动对应 `buzz-acp` Runner。
10. 验证 Runner 进程存活、Relay 连接建立且 Channel 成员关系完整。
11. 将状态原子更新为 `active`，再次同步 Desktop 镜像并输出非敏感摘要。

创建过程中不把 Agent 加入参数之外的 Channel。

## 8. 失败、回滚与幂等

### 8.1 Relay 写入前失败

删除本次新建的配置、工作目录和身份文件，并从注册表移除 `provisioning` 条目，不留下 Relay 成员关系。

### 8.2 部分 Channel 加入失败

撤销本次已经成功添加的 Channel 成员关系，再清理本地新建内容。若撤销失败，命令明确列出残留 Channel 并返回失败，不把状态标记为 `active`。

### 8.3 Runner 启动或连接验证失败

保留身份、配置和已确认的 Channel 成员关系，状态改为 `failed`。这样不会因自动清理造成身份漂移，也便于修复配置后恢复：

```text
platform/bin/resume-public-agent --id research
```

恢复命令重新校验注册表、身份、配置、显式 Channel 成员关系并启动 Runner；成功后状态改为 `active`。

### 8.4 重复执行

- 相同 `id` 且名称、公钥、Channel 和路径一致：只补齐缺失步骤并验证，不重复生成身份。
- 相同 `id` 但关键参数冲突：拒绝执行，要求使用独立的更新流程。
- 公钥已被其他 `id` 使用：拒绝执行。

## 9. 动态 Runner Supervisor

`agents-up`、`agents-status` 和 `start-agent` 从部署注册表发现 Agent，替代固定的五角色列表：

- `agents-up` 启动所有 `enabled=true` 且状态为 `active` 或 `failed` 的条目。
- `start-agent <id>` 校验注册表后只启动指定 Agent。
- `agents-status` 按注册表报告配置、tmux window、进程和 Relay 连接状态。
- `agents-down` 继续关闭本 PoC 的 Agent tmux session，但状态文件不被删除。

每个 Agent 使用独立 tmux window、身份文件、配置目录和工作目录。Supervisor 不读取注册表之外的目录来“猜测” Agent，避免残留文件被意外启动。

## 10. 现有五个 Agent 迁移

迁移脚本把 Product、Tech、Coding、CR、QA 写入注册表：

- 沿用现有公钥和 identity 文件；
- 沿用现有 `platform/agents/<role>/agent.env`；
- 沿用现有 `worktrees/<role>`；
- 显式记录当前两个 Channel；
- `source` 设为 `builtin`；
- 不重新生成身份，不更改 Channel 角色，不重建工作目录。

迁移后先以 dry-run 比较原固定清单和注册表解析结果，再切换动态 supervisor。五个 Runner 全部重新连接并完成 Mention/回复冒烟测试后，才删除脚本中的固定角色分支。

## 11. 安全边界

- 注册表和 Desktop 镜像不得包含 `nsec`、私钥、API Key、Token 或完整环境变量。
- identity 文件必须为 `0600`；包含身份目录的父目录不得对其他用户开放写权限。
- CLI 输出只显示 Agent ID、名称、公钥、Channel、状态和非敏感路径。
- 工作目录默认位于专用 Runner 根目录；自定义路径必须通过拒绝列表和规范化路径校验。
- Runner 不获得 Docker Socket、生产凭据、敏感用户目录或主干合并/发布权限。
- Channel 授权以 Relay 实际结果为准，本地注册表不能提升 Relay 权限。
- Desktop Tauri 命令只读取固定应用数据文件，不接受路径参数。

## 12. 测试策略

### 12.1 单元测试

- 注册表 schema、版本、重复 ID/公钥、空 Channel 和状态校验。
- Desktop 合并逻辑：注册 member 被识别，未注册 member 保持人员，Channel 不匹配不显示，kind `10100` 去重且优先。
- Desktop 缺文件、损坏 JSON、未知版本时的降级。
- CLI 参数解析、路径校验、镜像投影和敏感字段扫描。

### 12.2 集成测试

- 无 `--channel` 时创建被拒绝且无本地或 Relay 副作用。
- Channel 无效或 Owner 无权限时，在身份生成前失败。
- 多 Channel 中途失败时回滚已添加成员。
- Runner 启动失败时保留身份和成员关系并标记 `failed`。
- `resume-public-agent` 可把修复后的 Agent 恢复为 `active`。
- 重复执行相同命令不产生第二身份或重复成员。

### 12.3 回归与冒烟测试

- 现有五个 Agent 公钥、私钥文件、工作目录保持不变。
- 五个 Runner 均在线。
- 本机定制 Desktop Agents 模块显示五个公共 Relay Agent。
- 官方 v0.5.0 局域网客户端在两个指定 Channel 中都能 Mention 并收到回复。
- 新 Agent 只出现在显式指定的 Channel。
- 新增 Agent 后不重新打包 Desktop 即可显示。
- 注册表、Desktop 镜像和命令输出通过敏感信息扫描。

## 13. 验收标准

满足以下全部条件才视为完成：

1. `create-public-agent` 缺少 Channel 时无副作用地失败。
2. 无效或无权限 Channel 不产生身份、配置、工作目录、注册表条目或 Relay 残留。
3. 创建成功后 Agent 以 `member` 身份只加入指定 Channel。
4. 本机定制 Desktop 把已注册成员显示为 Relay Agent；未注册成员仍显示为人员。
5. `active` 与 `failed` 状态在 Desktop 中可区分，失败 Agent 显示离线。
6. 现有五个 Agent 身份和工作目录不变，均可 Mention、回复并由动态 supervisor 管理。
7. 局域网官方 v0.5.0 客户端仍能与五个 Agent 交互。
8. 注册表和 Desktop 镜像不含任何私钥或 Provider 凭据。
9. 后续新增公共基础 Agent 不需要修改或重新打包 Desktop。

## 14. 兼容性边界

本方案解决的是本机定制 Desktop 的 Agent 展示，以及官方 v0.5.0 客户端的 Channel Mention/交互兼容性。局域网其他成员使用的官方 Desktop 不会读取本机注册表，因此不承诺在其独立 Agents 模块中展示外部 Relay Agent；其受支持路径是从已授权 Channel 中 Mention 并交互。
