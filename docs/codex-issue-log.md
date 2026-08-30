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
- 版本/提交：`0.5.15-12` / `4d8997be`；测试包 `0.5.15-13` 基于同一提交重新构建。

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
- 版本/提交：`0.5.15-15` / `136a3cd1`；测试包已生成。

## 2026-08-29：Inbox Thread 上下文加载后页面定格

- 现象：打开 Inbox 中的 Thread 后一直显示 `Loading surrounding context...`，右侧页面像被定格。
- 定位：feed 轮询会替换选中 `FeedItem` 的对象引用，导致上下文 hydration effect 重复启动；Relay 重连等待或本地 `get_event` 请求没有统一总超时，`isLoading` 可能长期保持为 `true`。
- 处理：Thread 上下文改用事件字段作为稳定依赖，避免 feed 刷新重复启动请求；增加 15 秒总超时，超时后结束 loading 并显示上下文加载错误。
- 验证：`pnpm typecheck`、Biome 检查和启动日志单元测试通过；Thread 上下文回归测试待补充。
- 版本/提交：待提交。

## 2026-08-29：旧安装配置下 Thread 仍卡在加载

- 现象：使用旧安装配置打开 Inbox Thread 仍停在 `Loading surrounding context...`，而清空配置后的新环境正常。
- 定位：启动迁移只清理了 `buzz-channels.v1` 等目录索引缓存，却遗漏真正保存频道消息窗口的 `buzz-channel-messages.v1`；旧安装还会保留已经写入的 `buzz-storage-repair-v1` 标记，导致后续版本不会再次清理。
- 处理：增加 `buzz-storage-repair-v2` 迁移标记，并将频道消息快照纳入一次性清理范围。清理仅删除可重建的社区/频道缓存，保留 Community、身份、Agent 配置和服务端聊天记录。
- 验证：待运行桌面类型检查、Biome 和社区存储回归测试。

## 2026-08-29：旧配置仍可能恢复无效 Relay 状态

- 现象：清理频道缓存后，部分旧安装仍在打开 Inbox Thread 时卡住；截图同时显示 Thread 内容和 `Can't reach the relay`。
- 根因：旧 Community 记录中的公网/LAN 地址此前只做了宽松格式检查，失效的 LAN 地址、带路径/参数的 Relay 地址仍可能被恢复；冷启动 URL 中的旧 Thread 还会触发无超时的 `getEventById`。
- 处理：启动修复现在会丢弃空或带凭据、参数、路径的 Relay 配置，校验并规范 LAN 地址，失效 LAN 地址自动移除；冷启动 Thread anchor 增加 10 秒上限。
- 结果：旧配置即使存在，也会降级到可用的公网 Community 或可操作的错误状态，不再让单个旧请求锁住界面。

## 2026-08-29：离线时更新日志弹窗仍导致界面像卡住

- 现象：启动后更新日志弹窗出现，背景界面被锁定，Relay 断开时无法点击 Community 或重试控件。
- 根因：`StartupChangelogDialog` 虽然设置了 `modal={false}`，但共享 `DialogContent` 仍无条件创建全屏 `DialogOverlay`。透明 Overlay 仍覆盖整个 WebView，造成底层界面无法交互。
- 处理：为 `DialogContent` 增加 `showOverlay` 选项；启动更新日志使用 `showOverlay={false}`，同时保留普通设置对话框的默认模态遮罩。
- 结果：更新日志仍在启动时显示，但不再阻塞 Community/Relay 恢复操作；关闭按钮、右上角关闭和 Esc 仍可用。

### 追加定位：非模态 Dialog 仍参与全局交互管理

- 处理：启动更新日志不再使用 Radix Dialog，改为普通的非模态浮层；不创建 Overlay、FocusScope 或 DismissableLayer，仅浮层内容本身接收点击。

### 追加定位：日志列表滚动容器吞掉了关闭按钮

- 现象：更新日志可以上下滚动，但内容较长时“知道了”按钮位于滚动内容末尾，不在当前可视区域，用户感觉按钮点击无效。
- 处理：改为三行布局，标题固定、日志列表独立滚动、底部关闭按钮固定显示。

## 2026-08-29：LAN fast path 下更新日志可滚动但应用无法点击

- 现象：同一个 `0.5.15-20` 安装包在其他电脑正常；故障机启动后更新日志可以滚动，但“知道了”和其他 React 控件无响应，WebView 渲染进程持续占满一个 CPU 核心。
- 定位：两台电脑的 EXE SHA-256 完全一致。逐键 A/B 验证确认，仅当 active Community 同时保存公网 `relayUrl` 和 LAN `lanRelayUrl` 时复现。浏览器级 V8 trace 显示热点集中在 `subscribeLive -> subscribe -> ensureConnected` 和 presence subscription reconcile。提交拓扑确认 `Lin/develop` 从未包含上游 `6ea7a2b2`（#3320）的早到 Relay 帧缓冲修复；LAN fast path 并未删除该修复，而是让原有竞态更容易稳定触发。LAN 连接建立得更快，AUTH challenge 会在 `authRequest` 安装前到达并被丢弃，继而触发认证等待和订阅重试循环。公网连接较慢，所以问题具有机器和网络环境差异。
- 处理：恢复连接期间的入站帧缓冲；先安装 AUTH waiter，再按顺序排空早到帧；保留 LAN transport 和 canonical Relay 身份；增加缓冲顺序及溢出回归测试。
- 验证：Relay 单元测试 11/11、TypeScript typecheck、Biome、E2E build 均通过；两个确定性 early-AUTH E2E 均通过，覆盖首次认证/打开频道/首次发送，以及首轮 AUTH 签名挂起后超时重连；原有 initial-dial retry E2E 单独复跑通过；Tauri native WebSocket/LAN transport 测试 9/9 通过。仍需用保留原配置的安装包实机复测 CPU 与按钮交互。
- 版本/提交：待提交。

## 2026-08-30：点击“知道了”后进入 Community 卡死

- 现象：更新日志中的“知道了”现在可以点击，但进入 Community 后页面仍像卡死，旧配置/旧连接状态下尤其明显。
- 根因：Relay 认证成功后，`connect()` 仍等待旧连接的订阅回放和频道历史回补；回放会等待限流窗口、HTTP 修复请求或大量旧订阅。所有新挂载的 Community 查询共享同一个 `connectPromise`，因此会被旧回放串行阻塞，表现为进入后一直加载、消息框和页面交互不正常。
- 处理：认证成功即标记连接可用、启动连接看门狗并释放 `connectPromise`；订阅回放改为后台 best-effort 任务。新订阅立即发送自己的 REQ，回放期间若真实写入失败仍沿用原有连接重置/重连路径。若 LAN WebSocket 已完成握手但认证失败/超时，下一次自动重连单次跳过 LAN，改用公网 Relay；LAN 认证采用 8 秒上限，避免坏的内网端点长期占住启动流程。
- 验证：新增 E2E 回归场景，限流历史回补期间仍能发送新消息；待运行 Relay 重连用例、TypeScript 和 Biome 检查。
- 版本/提交：待提交。

## 2026-08-30：Community 需要 LAN/公网双地址探测与手动切换

- 需求：加入 Community 时可同时填写公网 Relay 地址和内网 Relay 地址；启动连接先验证内网，内网不可用时自动回退公网。已连接公网后，用户回到内网时需要一个按钮重新检测并自动切换到内网。
- 处理：保留公网地址作为 canonical Relay 身份和 AUTH 地址；native WebSocket 增加 LAN-only 探测配置与实际 transport 回报。Relay 客户端的探测会完成 WebSocket 握手和 NIP-42 AUTH，成功后保留现有 live subscriptions 切到 LAN，失败则立即改用公网并恢复正常 LAN-first/public-fallback 策略。Community 菜单新增“检测并切换到内网 Relay”按钮及检测中状态。
- 验证：TypeScript 类型检查、Biome、Tauri native WebSocket/LAN 单元测试通过；待构建本地安装包进行双地址实机验证。
- 版本/提交：待提交。

## 2026-08-30：实际位于内网但 Buzz 判断 LAN Relay 不可用

- 现象：客户端地址为 `192.168.191.102/24`，能够直连 `10.24.11.82:3000`，WebSocket 握手也能收到 AUTH challenge，但手动检测仍提示“内网不可用”。
- 根因：LAN fast path 通过明文 WebSocket 直连，同时保留公网 Host 进行 Community 租户绑定。Relay 因直连协议要求 NIP-42 `relay` 标签为 `ws://公网主机名`，客户端却一直按 canonical 公网配置签入 `wss://公网主机名`，严格认证因此返回 `auth-required: verification failed`。此前 UI 把握手、认证和网络失败统一显示成“内网不可用”，进一步掩盖了真实原因。
- 处理：LAN transport 的 AUTH 事件只把 canonical URL 的协议改为 `ws`，保留相同公网主机名和 Community 身份；公网 transport 继续使用原始 `wss`。手动检测结果现在区分 LAN 失败与公网回退失败，并显示 Relay 返回的具体错误。
- 验证：AUTH URL 与入站缓冲测试 5/5、TypeScript、Biome、Tauri native WebSocket 9/9、Relay NIP-42 9/9 通过。对真实 `10.24.11.82:3000` 保持公网 Host 的 A/B 探测中，`wss://公网主机名` 返回 `auth-required: verification failed`，`ws://同一公网主机名` 返回认证成功，直接确认根因与修复方向。待生成标识测试包进行实机验证。
- 版本/提交：`0.5.16` 本地发布；源代码提交见本次版本提交。
