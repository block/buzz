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
- 版本/提交：`0.5.18` / 本次修复提交。

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

## 2026-08-31：ngrok 公网入口达到额度且无法切回旧域名

- 现象：当前公网入口 `content-swift-seemingly.ngrok-free.app` 返回 HTTP 403 / `ERR_NGROK_727`，用户希望切回此前的 `fairy-sigilistic-elizbeth.ngrok-free.dev`。
- 诊断：当前 ngrok agent 在线，但账号已达到当月 HTTP 请求上限。旧域名返回 `ERR_NGROK_3200`；使用当前凭据启动旧域名时，ngrok 明确返回 `ERR_NGROK_320`，说明域名保留在另一个账号。切换前备份与当前 `ngrok.yml` 的 token 指纹一致，机器上没有保存可用于旧域名所属账号的另一套凭据。
- 处理：保留现有 ngrok 配置和进程，未贸然覆盖 token 或切换域名。要恢复旧域名，需要先取得该域名所属账号的 authtoken；也可换用一个未超额账号的新域名。
- 验证：本地 ngrok API 报告 agent `online`；当前入口稳定返回 `ERR_NGROK_727`，旧入口稳定返回离线错误，relay 本机及 LAN 服务不受影响。
- 版本/提交：诊断时仓库为 `7bb6bc01`（`0.5.16`）。

### 追加处理：恢复旧账号和旧域名

- 用户重新配置旧域名所属账号的 authtoken 后，`buzz-ngrok.service` 成功启动，`fairy-sigilistic-elizbeth.ngrok-free.dev/_readiness` 返回 HTTP 200。
- 将 Relay canonical URL 和 ACP 默认 URL 切回 `wss://fairy-sigilistic-elizbeth.ngrok-free.dev`，停止已超额的 `content-swift-seemingly.ngrok-free.app` 独立隧道，并完整重启 Relay。

## 2026-08-31：公网 IP 直连被识别为未配置社区

- 现象：客户端直连 `ws://121.192.177.100:3000` 时进入 Profile 创建流程，随后收到 `relay returned 404 Not Found: relay: no community is configured for this host`。
- 定位：relay 使用请求 Host 选择社区；现有社区主 Host 是 `fairy-sigilistic-elizbeth.ngrok-free.dev`，数据库没有公网 IP 的 Host 别名。另，服务器实际网卡地址是 `121.192.177.100/26`，不是用户提到的 `121.192.177.119`；后者在本机端口探测中连接被拒绝。由于端口为 3000，实际 Host 需要匹配 `121.192.177.100:3000`。
- 处理：将 `121.192.177.100` 和 `121.192.177.100:3000` 写入 `community_host_aliases`，均指向现有社区 `fairy-sigilistic-elizbeth.ngrok-free.dev`，不创建新社区、不迁移已有数据。
- 验证：通过 `Host: 121.192.177.100:3000` 的本地 NIP-11 请求返回 HTTP 200；canonical 域名仍返回 HTTP 200。无需重启 relay，Host 别名查询为请求级数据库读取。
- 版本/提交：运行时仓库为 `7bb6bc01`（`0.5.16`）。

## 2026-08-31：同一 Relay 经不同 Host 进入后 Profile 和历史记录分裂

- 现象：通过公网 IP 进入同一 relay 时要求重新建立 Profile，进入后看不到此前的频道和历史记录。
- 定位：数据库中存在两个独立 Community：`content-swift-seemingly.ngrok-free.app` 有 2715 条事件、101 个用户和 27 个频道；后来恢复的 `fairy-sigilistic-elizbeth.ngrok-free.dev` 被启动流程创建为新 Community，只有 71 条事件、9 个用户和 3 个频道。IP Host 别名最初指向后者。同一用户公钥在历史 Community 有 318 条事件、在新 Community 只有 17 条，直接证明数据被 Host 边界分开，而不是客户端丢失历史。
- 处理：保留两个 Community 的数据；将原 `fairy` 小 Community 改名为 `legacy-fairy-sigilistic-elizbeth.ngrok-free.dev`，将历史 `content-swift` Community 的主 Host 改为当前 `fairy` 域名，并把 `content-swift`、`121.192.177.100`、`121.192.177.100:3000`、`121.192.177.119`、`121.192.177.119:3000` 全部设为历史 Community 的别名。完整重启 relay 后，部署 Community 确认为历史 Community id `0a3b5be4-3092-4ac9-99a7-be8de0892bcb`。
- 验证：canonical Host 和 IP Host 的本地 readiness 均返回 HTTP 200；relay 启动日志显示 `Deployment community ensured` 指向历史 Community。公网 ngrok transient unit 同时消失，因此补建持久化 `buzz-ngrok.service` 后单独验证公网入口。
- 版本/提交：运行时仓库为 `7bb6bc01`（`0.5.16`）。

## 2026-08-31：工作站绑定公网 IPv4 并保留内网连接

- 现象：用户希望在保留内网地址 `10.24.11.82` 可连接的同时，为工作站绑定公网地址 `121.192.177.100/26`，公网网关为 `121.192.177.65`。
- 定位：当前唯一有链路的网卡为 `ens1f3`，已有 `10.24.11.82/26` 和默认网关 `10.24.11.65`；其他网口均无载波。公网地址可作为同一网卡的第二地址，但两个网关不能直接并列为默认路由，应为公网源地址配置独立路由表和策略规则。
- 处理：完成 NetworkManager 配置语法校验，方案为保留内网默认路由，增加公网地址、`table 177` 的公网直连/默认路由，以及 `from 121.192.177.100/32` 的策略规则。尝试修改现有连接时因当前会话没有管理员授权而返回 `Insufficient privileges`，未改变系统网络配置。
- 验证：只读检查确认当前内网地址和路由保持不变；公网网关连通性尚未验证，需管理员应用配置后测试 ARP/ICMP 和实际服务端口。
- 版本/提交：工作区 `Lin/develop`，未创建提交。

### 追加验证

- 用户已用管理员权限成功执行 NetworkManager 配置并重新应用到 `ens1f3`。
- `10.24.11.82/26` 与 `121.192.177.100/26` 同时存在；`ip rule` 显示 `from 121.192.177.100 lookup 177`；公网流量解析为经 `121.192.177.65` 出口。
- 公网网关 `121.192.177.65` ping 3/3 成功，延迟约 0.84–4.01 ms；内网默认路由未被替换。

## 2026-09-01：删除 Custom Agent 后人格卡片重新出现在 Agents 区域

- 现象：删除下方 Custom agents 中的 Agent 后，同名卡片重新出现在上方 Agents 区域。
- 定位：旧版 Codex task Agent 记录可能带有迁移生成的 `personaId`。删除实例只移除了 managed-agent 记录，但仍保持该人格定义为 active；统一列表刷新后，失去任务实例过滤条件的定义被重新渲染为普通 Agents 卡片。
- 处理：删除绑定 Codex task 的最后一个实例时，仅对非内置、非团队、未共享且非共享目录副本的人格自动设为 inactive；普通自定义人格、内置人格和仍被其他实例使用的人格不受影响。删除完成后同时刷新 managed agents、relay agents 和 personas 查询。
- 验证：待运行 Tauri Rust 测试、桌面 TypeScript/格式检查，并验证删除任务 Agent 后不会出现重复卡片。
- 版本/提交：待提交。

## 2026-09-01：Codex Code Mode host 缺失导致运行时警告

- 现象：Agent Activity 中出现 `Code Mode is unavailable because failed to spawn code-mode host ...\\codex-code-mode-host.exe: host executable was not found`，但普通 Codex 对话仍可能继续返回结果。
- 定位：Windows Codex 更新会在 `%LOCALAPPDATA%\\OpenAI\\Codex\\bin` 中短暂或长期留下只有 `codex.exe` 的版本目录；Buzz 之前始终传入 `-c features.code_mode_host=true`，Codex 随后按当前运行时版本哈希查找同目录 sidecar，找不到就把该警告作为 Agent 输出返回。该 sidecar 是 Codex Desktop/CLI 的可选安装组件，不属于 Buzz 安装包，Buzz 不应假定每台机器都有它。
- 处理：将 app-server 主程序和 Code Mode host 解耦探测。只要 `codex.exe` 存在即可启动共享 app-server；仅在同目录存在匹配 `codex-code-mode-host(.exe)` 时传入 `features.code_mode_host=true`。启动日志新增 `code_mode_host=enabled|unavailable`，便于区分可选能力缺失与运行时本身启动失败。
- 验证：Tauri Codex runtime 单元测试 9/9、桌面 TypeScript typecheck 通过；本次文件 rustfmt 通过。缺少 sidecar 时验证不再带 Code Mode 开关，有 sidecar 时保持原行为。
- 版本/提交：`0.5.18` / `45e2e9b4`。

## 2026-09-02：Android 附件发送长时间停留在 Sending

- 现象：Android 端发送一个很小的附件仍需约 2 秒，早前个别发送接近 7.8 秒；分阶段日志显示读取约 220 ms、上传约 1.11 秒、消息发布确认约 599 ms。
- 定位：附件字节和 relay 服务端不是主要瓶颈。平板连接名为 `ASUS_5G` 的网络时实际工作在 2412 MHz，直连 relay 的请求在 136 ms 至 2.1 秒之间抖动；relay 本机处理消息仅约 21 ms，MinIO 健康检查约 5.5 ms。切换到 5220 MHz 网络后，连续请求稳定在 98 至 119 ms。客户端 publish 路径和 relay 出站路径均没有固定 500 ms 等待。
- 处理：保留文件读取、上传、签名和 relay 确认的分阶段诊断日志；移动端媒体上传优先使用已验证的 LAN transport，并复用 HTTP client；新媒体上传先检查租户 sidecar，sidecar 不存在时跳过无意义的 blob `HEAD`。测试过 Android Wi-Fi low-latency lock，但没有稳定收益，未纳入产品代码。
- 验证：真实附件发送降至约 1.13 秒，其中读取 57 ms、上传 412 ms、消息准备/签名/确认 650 ms；`buzz-media` 121 项测试和移动端相关 123 项测试通过，`flutter analyze` 无问题，debug APK 已在实机安装验证。
- 版本/提交：基于 `047c8141`，本次 PR 待提交。

## 2026-09-02：登录后 Inbox 线程部分上下文未加载

- 现象：登录后打开 Home/Inbox 的线程详情，消息主体可以显示，但顶部出现 `Some message context could not be loaded.`，部分根消息、父消息或上下文没有补齐。
- 定位：Inbox 冷启动会通过 `get_event` 按事件 ID补拉线程祖先；桌面 Tauri 命令此前只查询少量 kind，遗漏了 legacy stream、job 生命周期、workflow、Git、forum vote 及 huddle 生命周期等公开事件。另一个问题是请求可能发生在首次 AUTH 或重连窗口，临时 relay 失败会被永久记录为加载错误。
- 处理：新增明确的公开 `EVENT_LOOKUP_KINDS` 集合，覆盖可作为线程根/父事件的公开类型，同时排除 recipient-gated、author-only 和其他受限事件，避免放宽 relay 访问边界；Inbox 上下文加载现在监听 relay 连接状态，非 `connected` 阶段的失败不再显示为持久错误，并会在连接认证成功后重新 hydration。
- 验证：新增 Rust 单元测试锁定关键公开 kind 均可查找且 lookup filter 不含 p-gated kind；消息命令测试 13/13、Inbox 辅助测试 23/23、桌面 TypeScript、Biome、E2E 构建和本次文件 rustfmt 检查通过。全量桌面测试共 4,883 个用例，其中 4,882 个通过、1 个与本次改动无关的既有失败。
- 版本/提交：待提交。

## 2026-09-02：Buzz 退出后遗留 Codex shared app-server

- 现象：Buzz 启动的 `codex.exe app-server --listen ws://127.0.0.1:51919` 在 Buzz 父进程退出后继续运行，锁住 Codex Desktop 管理的运行时目录；后续 Codex 更新因 `EPERM` 无法刷新二进制，并误报 `Unable to locate the Codex CLI binary`。
- 定位：`spawn_codex_shared_runtime` 启动子进程并记录 PID 后立即丢弃 `Child`，该进程既不在 managed-agent 进程表中，也未绑定 Windows kill-on-close Job Object，应用退出路径因此无法回收它。现场确认遗留 PID `35120` 的父进程已经退出，执行路径位于 ChatGPT 管理的 Codex runtime，监听端口为 `51919`。
- 处理：Buzz 现在只登记自己实际启动的 shared app-server，保留其 `Child`；Windows 下立即绑定 kill-on-close Job Object，正常退出/重启时在 managed agents 停止后显式回收整棵进程树，异常退出时由 Job Object 回收。连接到启动前已存在的外部 shared server 时不登记所有权，也不会在 Buzz 退出时终止它。启动探测失败时立即回收本次启动的进程。
- 验证：新增 Windows Job Object 回归测试，确认关闭所有权句柄后子进程在 2 秒内退出；该测试 1/1、既有 Codex shared-runtime 测试 9/9 通过，三个修改的 Rust 文件 `rustfmt --check` 和 `git diff --check` 通过。Tauri lib 全量测试 2,357 通过、9 失败、11 ignored；9 个失败来自 Windows 环境缺少 Unix `true` 命令及既有 provider fixture/egress inventory 漂移，均不涉及本次生命周期文件。
- 版本/提交：分支 `codex/codex-lifecycle-cleanup`，提交见本条所在提交。

## 2026-09-02：修正 Codex shared runtime 所有权，并恢复 Windows Agent Stop

- 现象：上一条未合并方案会在 Buzz 退出时一并终止共享 app-server，导致仍打开的 Codex Desktop 静默失去 backend；同时 Windows 上点击 Codex task Agent 的 `Disconnect Buzz` 可能无法及时释放 task。
- 定位：shared app-server 是 Buzz 与 Codex Desktop 共用的电脑级服务，不应归 Buzz 窗口所有；真正需要避免的是它长期锁住 Codex Desktop、ChatGPT 或 Scientist 的可更新 runtime 目录。另查明 `stop_managed_agent_runtime` 使用的 `process_is_running` 在 Windows 固定返回 `false`，会跳过已跟踪 harness 的终止步骤并直接等待仍在运行的子进程。
- 处理：推翻上一条未合并的 kill-on-quit 方案。Windows 首次启动 shared app-server 前，将 `codex.exe`、Code Mode host、command runner 和 sandbox setup 复制到 Buzz 管理的版本化不可变目录；成功启动后主动释放 `Child` 句柄，让 backend 跨 Buzz 退出继续服务，而 Codex 更新不再被锁。启动探测失败时仍只回收本次新进程。显式 Stop 改为对已跟踪 `Child` 调用 `try_wait`，仍运行时终止整棵进程树并等待退出，然后清理 receipt/session cache；Agent 身份和 Codex task 绑定保持不变。
- 验证：Codex lifecycle Rust 测试 11/11、runtime command 测试 10/10、前端 Agent Connect/Disconnect 行为测试 8/8 通过。新增测试确认版本更新产生新缓存且旧版本保持可执行、Buzz 丢弃子进程句柄后 backend 仍存活，以及 Windows Stop 确实终止运行中的进程树。`managed_agents` 扩展测试 980 通过、5 失败；失败均为 Windows 环境无法启动 Unix `env`/`true` 或 PATH 占位命令的既有跨平台夹具，不涉及本次修改路径。
- 版本/提交：分支 `codex/codex-lifecycle-cleanup`，待提交。

## 2026-09-05：新版 Codex Desktop backend 路径导致 Start 接管检查漏检

- 现象：Xiaoxin 同时运行 Codex Desktop 私有 app-server 与 Buzz shared app-server 时，本地 task Agent 点击 Connect Buzz 后直接进入监听状态，没有先显示 `Close and reconnect`。由于 task-bound Agent 使用 lazy worker，冲突会被延迟到首条工作消息才暴露。
- 定位：Buzz 只把 Windows Appx 安装目录下的 `app/resources/codex.exe` 识别为 Desktop backend；新版 Codex Desktop 实际从 `%LOCALAPPDATA%/OpenAI/Codex/bin/<version>/codex.exe` 启动 app-server。现场 PID `20536` 是 `ChatGPT.exe` 的子进程，但因路径不同未进入 `private_app_server_process_ids`。
- 处理：Windows 进程分类同时识别已验证 Desktop 进程树中的 `codex.exe app-server` 后代，并保留旧 Appx backend 路径匹配；监听 shared URL 的 backend 继续排除，独立 CLI app-server 也不会因路径相似被误判。这样 Start/Restart 会在创建 Agent worker 前触发现有接管确认。
- 验证：Tauri `codex_desktop` 定向测试 11/11 通过，覆盖 LocalAppData Desktop 子进程、旧 Appx backend、shared listener 与无关 CLI backend。随后构建并在 Xiaoxin 安装 `0.5.18-local.1`：新版准确显示 1 个私有 app-server，Connect 前弹出 `Close and reconnect`，且确认前没有启动 `buzz-acp`；确认后旧私有 PID 消失，Codex Desktop 重开并复用原 shared PID `2596`。临时 Agent 经 DM 返回精确文本 `BUZZ_LOCK_LIVE_OK`，日志记录 5 秒后 lazy worker 回收且没有 writer conflict；Disconnect 后 harness 退出，Desktop 与 shared runtime 保持运行。
- 版本/提交：PR 分支 `codex/detect-versioned-codex-desktop`；Xiaoxin 验收安装包基于原始测试提交 `9760ec38`，版本 `0.5.18-local.1_9760ec382776`。

## 2026-09-05：Windows 退出登录被长期 shared app-server 日志锁阻塞

- 现象：`59.77.33.59:6000` 的 Buzz 在用户点击退出登录并重启后挂住，持续保留 `.xyz.chemyibinjiang.buzz.codexlab.reset-pending`；stderr 报错为无法将整个 app-data 目录重命名为 `.reset-trash`，Windows 返回 `Access is denied (os error 5)`。
- 定位：退出登录已经正确写入 reset sentinel，问题发生在下一次启动的两阶段清理。Buzz 有意让 Codex shared app-server 跨窗口退出继续运行，但旧版把该进程的 stdout/stderr 放在 app-data 下；现场独占锁探针确认只有 `agents/logs/codex-shared-runtime.stdout.log` 和 `codex-shared-runtime.stderr.log` 被长期 app-server 占用，因此 Windows 不允许重命名其父目录。shared backend 的生命周期策略正确，错误在于把电脑级服务日志放进了身份级 reset 边界。
- 处理：Windows 新启动的 shared runtime 日志迁移到 app-data 同级的隐藏目录，避免长期进程锁住退出登录清理目标；错误诊断仍回退读取旧日志。为兼容已经运行旧 backend 的机器，整目录原子重命名失败时仅允许保留上述两个精确日志路径，其余数据先递归移动到 rollback trash，keychain 删除成功后清除，失败则完整恢复；任何其他锁定文件仍会失败并回滚。
- 验证：新增 Windows 独占句柄回归测试，确认旧 shared-runtime 日志保持打开时身份数据仍被删除、sentinel 清除且 backend 无需退出；另一个测试确认 keychain 失败会恢复设置并保留 sentinel。reset 聚焦测试 16/16、Windows shared-runtime 聚焦测试 4/4 通过。随后在原问题机器安装 `0.5.18-local.2_c0572483dd83`：先用非交互 SSH 会话验证 keyring 不可用时全部数据正确回滚；再从已登录桌面会话启动，reset sentinel 和 rollback trash 均被清除，旧 Agent PID receipts 归零，两份锁定日志保留，shared app-server 始终保持原 PID `32764` 且 readiness 返回 HTTP 200。
- 版本/提交：分支 `codex/windows-signout-shared-runtime-logs`；代码提交 `c0572483`，远端验收安装包 SHA-256 `69e5dcdd35a4d8aa81cb8e86ff0c8464b3698cb63f0cd4977af5d9a136b26071`。
