# Buzz Codex Lab 问题记录

这个文件记录用户反馈的问题、定位结论、修复内容和验证版本。每次处理新的用户问题时都要追加一条记录，不要覆盖历史记录。

## 记录格式

```markdown
## YYYY-MM-DD：问题标题

- 现象：
- 定位：
- 处理：
- 验证：
- 版本/提交：
```

## 2026-08-29：公网 Community 频道无法连接

- 现象：Buzz 已能正常启动，但同时配置公网 Relay 和 LAN Relay 时，公网 Community 的频道无法连接；使用 `d1c5eca5` 的其他电脑可以正常连接。
- 定位：公网 Relay 的 HTTP 和 WebSocket 服务均可用。HTTP `/query`、`/events` 请求错误地直接使用 LAN Host，Relay 按 Host 识别为另一 Community，导致公网 Community 的频道查询和消息发送落到错误租户。
- 处理：恢复 HTTP 查询和事件提交使用 canonical/public Relay URL；LAN 地址仅作为原生 WebSocket 的传输拨号地址，并保留公网 Host 作为 Community 身份。
- 验证：前端 TypeScript 类型检查通过；相关 Rust Relay 测试通过（本机两个网络监听测试因 Windows 套接字错误 `10055` 失败，与本次逻辑无关）。
- 版本/提交：`0.5.15-12` / `4d8997be`；测试包 `0.5.15-13` 基于同一提交重新构建。

## 2026-08-28：启动后一直加载

- 现象：正式应用标识下启动卡在加载界面，独立诊断标识正常；同一安装包在其他电脑正常。
- 定位：原应用标识的 WebView2 持久化状态包含失效的 active Community、旧 Relay 和频道缓存；Relay 后台日志仍显示可连接，因此不是服务端或网络故障。
- 处理：启动时修复失效 Community、清理过期 Relay/频道缓存，并为阻塞的启动阶段增加超时恢复界面；保留身份、Agent 配置和聊天记录。
- 验证：Community 测试 `14/14`、TypeScript 类型检查和格式检查通过。
- 版本/提交：`0.5.15-11` / `c8f21fc9`。

## 2026-08-29：公网 Community 无法连接并卡在加载界面

- 现象：点击公网 Community 后持续显示加载状态，无法点击其他位置；公网频道仍无法连接，内网 Community 不受影响。
- 定位：社区切换后的身份、资料和频道初始化依赖 Relay HTTP 请求。公网请求经过代理或连接半开时，旧客户端没有请求级超时，导致 Promise 长时间 pending，React 的 Community/Onboarding gate 无法释放。此前为避免 LAN Host 造成租户错配而移除 HTTP fallback，也同时失去了对代理异常的恢复路径。
- 处理：为 Relay HTTP 客户端增加 15 秒请求超时；公网 canonical URL 请求失败时仅切换到直连客户端重试，保持相同 URL、Host 和 NIP-98 `u` 标签，不把 LAN 地址当作另一个 Community；提交事件沿用同样的直连重试策略。
- 验证：`pnpm typecheck`、Biome 检查、`cargo check` 和 Relay 单元测试（30/30）通过。全量 `cargo fmt --check` 仍被工作区已有的未格式化文件阻塞，本轮改动文件本身未引入格式问题。
- 版本/提交：待提交。

## 2026-08-29：Relay 断开时无法退出 Community

- 现象：公网或内网 Relay 断开后，点击退出 Community 会一直等待，无法完成本地移除。
- 定位：退出流程先等待 NIP-43 leave 请求被 Relay 接受；`leaveCommunity` 的网络超时直接向上抛出，`communities.removeCommunity` 因此不会执行。
- 处理：将超时、Relay unreachable、WebSocket/网络连接失败归类为连接故障，连接故障时继续完成本地退出；权限拒绝、协议错误等非连接错误仍然保留原有阻止退出行为。
- 验证：退出流程相关单元测试 8/8 通过，TypeScript 类型检查和 Biome 检查通过；全量桌面测试有 1 个既有环境相关失败，其余 4873 项通过。
- 版本/提交：待提交。

## 2026-08-29：启动更新日志弹窗卡住应用

- 现象：启动后更新日志弹窗覆盖整个应用，关闭后在启动状态重新渲染时可能再次出现，用户感觉 Buzz 一直卡在更新日志页面。
- 定位：`StartupChangelogDialog` 原先只判断 Community 配置已应用，不判断 AppReady 是否完成或 Relay 是否在线。因此公网 Relay 断连、缓存中的旧 Community 被恢复时，日志弹窗仍会覆盖启动恢复界面；同时组件重新挂载会把 `open` 重置为 `true`。
- 处理：将弹窗移动到 AppReady 完成后的渲染分支；Relay 未连接时使用非阻塞日志层，让底层重试/更换 Community 控件可操作；关闭按钮改为显式更新 Dialog 状态，并在进程级记录已关闭状态，覆盖按钮、右上角关闭和 Esc 关闭路径。
- 验证：`pnpm typecheck`、Biome 检查和现有启动日志单元测试通过。
- 版本/提交：待提交。
