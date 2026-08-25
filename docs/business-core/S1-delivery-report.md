# Business Core S1 Delivery Report

## 1. Decision

`S1_CONDITIONALLY_READY`.

S1 代码、静态门禁、Web 构建、Dock 资源解析、一次性 PostgreSQL 工作流、代表性
数据量 HTTP 基线、真实 Authentik 浏览器链路，以及包含自然过期恢复的交互式 macOS
打包验收通过。Windows WebView2 打包验收尚未完成，因此不声明跨平台
production-ready。

## 2. Delivered

- 新增范围内经营驾驶舱：销售履约、采购到货、库存结构、真实利润。
- 新增数据质量中心：库存、经营应收、经营应付、利润事实四域对账。
- 新增投影 worker 新鲜度、待消费事件、失败和事实水位状态。
- 新增 Business Web 驾驶舱和数据质量页面。
- 新增 Business Dock singleton 深链并拒绝额外路径、query 和 traversal。
- 新增 Read API/MCP 只读工具 `get_operating_dashboard`、
  `get_business_data_quality`；固定只读工具总数从 26 增至 28。
- 新增 `just business-s1-check`。

## 3. Correctness changes

利润投影 offset 现在每次 worker heartbeat（包括无新事件的空轮询）和 rebuild 都更新
`updated_at` 并增加 `version`。原实现只推进事件位置，长期无新出库时会被稳定性检查
误判为过期水位。

经营指标不跨计量单位相加：采购到货率按完成采购行计算；库存展示库位商品计数，
不把不同 SKU/UOM 的数量合成一个无意义总数。金额始终限制单币种。

## 4. Authorization and safety

- 服务路径继续要求 service credential、audience、enterprise user 和 trace context。
- 浏览器路径继续复用 BusinessSession 中间件；GET 不扩大写权限。
- S1 要求 `management_report:read`，并应用全部 B1 数据范围。
- 恢复策略只允许证据检查、领域冲销和幂等投影重放；没有数据库修复写端点。
- MCP 保持只读、delegation-bound、固定 schema 和 allowlisted `biz://`。

## 5. Verification evidence

- `cargo clippy`（core/read-api/read-mcp/query-contracts，`-D warnings`）：PASS。
- Rust 单元与 Read API/MCP/contracts 测试：PASS。
- Business Web TypeScript + Vite build：PASS。
- Business Dock resolver：17/17 PASS。
- Disposable PostgreSQL B4+S1 workflow：PASS，0.74s。
- PostgreSQL 覆盖两笔销售订单、出库、成本、投影、费用分配、并发过账、冲销、
  快照、驾驶舱和数据质量；并验证人为库存漂移会立即进入 `blocked`，恢复后回到
  `complete`，无权限用户读取失败。最终差异、积压、失败均为零。

## 6. HTTP baseline

在同一小型一次性数据集、loopback、debug Business Core 进程上各发送 100 次真实服务
身份请求：

- Operating dashboard：P50 8.659ms，P95 12.238ms，max 34.181ms。
- Data quality：P50 8.611ms，P95 11.874ms，max 35.652ms。

这证明实际 HTTP/授权/SQL/序列化路径工作且远低于建议的本地门槛，但数据集只有两笔
销售订单；代表性容量证据见下节。

### S1.2 representative capacity baseline (2026-08-21)

在隔离、空白且名称包含 `s1_capacity` 的一次性 PostgreSQL 数据库中，通过真实 Axum
loopback HTTP、服务身份、audience、enterprise user 和 trace headers，写入并验证：

- 20,000 笔销售订单；
- 12,000 笔已确认出库；
- 10,000 笔采购订单，其中 6,000 笔已到货；
- 4,999 个 SKU 库位及匹配的库存流水/余额；
- 24,000 条利润事实。

预热 10 次后，每条路由连续采样 200 次：

- Operating dashboard：P50 51.504ms，P95 59.525ms，max 107.040ms；门槛 500ms。
- Data quality：P50 35.438ms，P95 41.039ms，max 54.951ms；门槛 2s。

两条路由均返回预期计数，数据质量为 `complete` 且 `differenceCount=0`。测试入口为
`postgres_s1_capacity` ignored integration test；它强制使用专用空库，拒绝普通数据库。
测试后的合成数据库已删除。

## 7. Remaining acceptance

- Windows WebView2 打包验收与恢复演练；当前 macOS arm64 主机无 Windows、WebView2
  或 Windows VM runner，不能用 Linux 容器结果替代。

### S1.1 real-environment progress (2026-08-21)

- 真实 Authentik、Business Auth Gateway、Business Core、Business Web 和本地 TLS
  Caddy 已在同一 PostgreSQL 实例上联通；未登录的 Business API 请求返回
  `401 business_session_required`。
- Chrome 已完成真实 Authentik PKCE SSO，并在网关产生活动 Workbench 会话；当前
  Buzz Dev 设备绑定为唯一活动绑定，旧设备绑定已原子撤销，关联 BusinessSession、
  embed session 和 agent delegation 同步失效。
- `Buzz Dev.app` release bundle 构建通过；bundle id 为
  `xyz.block.buzz.app.dev`，Info.plist 仅注册 `buzz-dev`。Chrome 的真实 OIDC 回调已由
  macOS 明确提示并成功交回开发包，避免误启动安装在 `/Applications` 的正式 Buzz。
- Authentik Workbench provider 同时严格登记正式版 `buzz://` 与开发版
  `buzz-dev://` 的登录/登出回调；未放宽到通配符。
- 补齐界面已宣告但原先未实现的 `⌘⇧B / Ctrl+Shift+B` Business Dock 快捷键；单测
  2/2、OIDC 配置测试 5/5、Desktop TypeScript 与 Biome 检查通过。
- 经用户授权，为 Buzz Dev 创建并绑定本地开发身份；未读取或记录私钥、会话 Cookie、
  embed code 或 CSRF token。为保持可恢复性，开发包旧 WebView 缓存移动至
  `/tmp/paqiaoli-buzz-dev-cache.YOvvEq`，未修改正式版 Buzz 数据。
- Business Web 实现有界 Business Bridge V3：校验 `event.source`、协议、版本、命令、
  nonce 和最大消息体；支持 Tauri opaque-origin 回退，并在 iframe 加载竞态前连接。
- 第三方 WebView 会话 Cookie 使用 host-only、`Secure`、`HttpOnly`、`SameSite=None`、
  `Partitioned`；`/api/session` 按读取轮换并返回只存在于内存的 CSRF token，写命令仍
  强制精确 Origin 与 CSRF 双重校验。
- `Buzz Dev.app` 中完成一次真实采购单写入：`PO-202608-000001`，状态 `draft`、
  `unreceived`、1 行；净额 CNY 250.00、税额 CNY 32.50、含税 CNY 282.50。数据库按
  `supplier_reference=S1-REAL-20260821-001` 核验恰好 1 条，事件 `created/version=1`。
- PostgreSQL 浏览器安全集成测试验证错误 Origin、缺失 CSRF、错误 CSRF 均返回 403，
  正确组合能通过安全中间件；网关重绑定、一次性 ticket、防重放、级联撤销、审计和
  agent delegation 的真实 PostgreSQL 测试通过。
- Business Web Bridge 5/5（含 CSRF logout）、生产构建、Business Core PostgreSQL 浏览器安全测试、
  Business Core Clippy、Business Auth Gateway 单元/JWT/PostgreSQL 测试均通过。

### S1.2 packaged-session progress (2026-08-21)

- macOS `Buzz Dev.app` 实机执行 Business-only logout：Workbench 会话保持活动，
  BusinessSession 被撤销；点击 Continue SSO 后复用现有 Authentik SSO，无第二次密码
  提示并恢复经营驾驶舱。
- 实机演练发现 Business Dock 仅在 iframe load 或手动重试时检查会话，已认证页面在
  后台自然过期时不会主动获知。现增加仅在 Dock 打开且已认证时运行的 15 秒
  `CHECK_AUTH` 心跳，关闭或离开认证态即清理定时器。
- 心跳单测 2/2、Business Dock Playwright 10/10、Desktop TypeScript、Biome 和新的
  `Buzz Dev.app` release bundle 构建通过。
- 用户在本机完成新 ad-hoc bundle 的 macOS 钥匙串授权后，实包自然过期复验通过：
  将唯一活动 BusinessSession 的到期时间推进到过去后，心跳在一个周期内发现失效，
  原会话进入 `expired`，系统只创建一个新的活动 BusinessSession 并恢复驾驶舱；
  Workbench 会话和设备绑定全程保持活动，无点击、浏览器弹窗或第二次密码提示。

### S1.3 operating-report observability (2026-08-21)

- 经营驾驶舱与数据质量响应新增逐阶段耗时、总耗时、目标耗时、最慢阶段和运行追踪
  元数据；读取超过 500ms/2s 目标时返回 `slow` 并写入结构化告警日志。
- 驾驶舱新增报表健康、水位新鲜度和结构化异常；数据质量中心统一暴露对账差异、投影
  失败、积压、worker 停用、水位过期和慢查询告警。恢复入口只指向证据与安全操作手册，
  不自动改写业务事实。
- 月度经营查询改为日期范围条件，并新增 7 个针对销售、出库、采购、利润事实、投影失败
  和 outbox 的局部/复合索引；没有引入会计、总账或法定财务模型。
- 隔离 PostgreSQL B4→S1.3 全工作流通过，并验证 7 个索引、健康响应、异常生成和恢复。
- 代表性容量库保持 20,000 销售订单、12,000 出库、10,000 采购订单（6,000 已到货）、
  4,999 个 SKU 库位和 24,000 条利润事实；预热 10 次后每条路由采样 200 次：
  Operating dashboard P50 46.994ms、P95 51.228ms、max 58.915ms（目标 500ms）；
  Data quality P50 32.684ms、P95 35.142ms、max 39.039ms（目标 2s）。
- `Buzz Dev.app` 真实 Authentik/BusinessSession/Dock 链路复验通过：经营驾驶舱状态
  `COMPLETE`、新鲜度 1 秒、读取 44.0ms、无活动异常；数据质量页四域均
  `CONSISTENT`，差异/积压/失败均为 0，读取 28.1ms。

### S1.4 operating-incident lifecycle (2026-08-21)

- 新增按授权范围隔离的经营事件簿：结构化告警可创建、条件清除、重开、认领、确认、
  开始处理、调整时限和解决。
- `critical` 默认 4 小时、`warning` 默认 24 小时；所有命令要求独立写权限、幂等键和
  乐观并发版本。
- 底层条件仍生效时数据库服务拒绝解决；重新扫描确认条件清除后才能完成。异常再次
  出现会重开原事件并保留发生次数和完整历史。
- 每次变更同时写入不可修改的事件轨迹和 Business Core 追加式审计，不提供自动修复或
  直接改写业务事实的入口。
- Business Web 新增“异常处置”事件簿，突出 SLA 时钟、负责人、状态、证据和处置轨迹；
  Business Dock 新增 `biz://operating-incidents` 单例深链。
- 真实 `Buzz Dev.app` / Authentik / BusinessSession 链路完成一次健康扫描：事件簿成功
  返回新增 0、重开 0、条件清除 0，数据库确认事件数 0、扫描审计数 1；未制造或改写
  真实业务差异。现有 `s1_operator` 获得独立处置权限，授权变更另有审计记录。

### S1.5 operating cadence and subscriptions (2026-08-22)

- 新增不可变经营日报/周报，按授权范围、周期、币种冻结销售、采购、库存时点、管理经营
  利润及异常 SLA 指标；重复生成返回同一证据，不覆盖历史。
- 趋势读取提供相邻周期变化，零基线保持空值；异常趋势复用 S1.4 权威事件，不另建第二套
  处置状态。
- 新增 Business Dock 站内订阅，支持每日/每周 08:00 计划、暂停/恢复、原子领取、失败
  退避、乐观并发、幂等和追加式事件审计。
- Business Web 新增“经营刻度尺”视图；Business Dock 新增
  `biz://operating-trends` 单例深链。
- 真实 `Buzz Dev.app` / Authentik / BusinessSession 链路已冻结 2026-08-21 日报：采购额
  CNY 282.500000、SLA 超时 0、质量 `complete`；随后启用每日 08:00 站内订阅，数据库
  确认快照 1、活动订阅 1、订阅事件 1、对应业务审计 2。没有制造合成业务单据。
- 明确未加入邮件/webhook 外发，也未加入凭证、总账、税务或法定财务报表。

## 8. Explicit exclusions

没有启动 B5。没有新增电子发票、银行流水、会计凭证、总账、税务、法定利润或
财务核算接口。
