# GLOSSARY.zh-Hans — 简体中文术语冻结表

规格 `BUZZ-DESKTOP-I18N-ZH-HANS-001` §8（术语）、NFR-002（目录一致性）、FR-010（禁译内容）、
§7 PR6。本表是 **冻结表**：所有表面必须使用同一译法；改动术语必须一次性同步
glossary、`en.json` / `zh-Hans.json`、`STRINGS-MANIFEST.md` 和中英对照截图，
不得由单个表面自行选择译名。

除标注「待审」的行以外，下表全部由 PR6 从 `desktop/src/locales/zh-Hans.json`
的 1160 条现有值中统计得出（不是外部词表），括号内是目录中出现的次数或证据。
PR6 全量审校时仍需至少一名母语 reviewer 签字。

## 1. 保留英文：品牌、协议与技术标识

品牌与协议名永不翻译、永不改写大小写。目录里有 54 条中文值按此规则保留了英文词。

| 类别 | 保留原文 | 规则 |
|---|---|---|
| 品牌 | `Buzz`、`Buzz Desktop`、`Pulse`（功能名）、`Builderlab`、`KLIPY` | 产品/公司名不译；`Pulse` 在 UI 文案中作“动态”，功能名位置保留英文 |
| 协议 | `Nostr`、`NIP-xx`（如 `NIP-05`、`NIP-49`）、`npub`、`nsec`、`ncryptsec`、`nprofile`、`nevent`、`naddr`、`pubkey`、`kind`、`ACP`、`MCP`、`JSON-RPC` | 协议标识与密钥前缀不译；短标签写「中继」，解释性文案写「中继服务器」 |
| 上游/第三方 | `GitHub`、`GitLab`、`Git`、`Tauri`、`Node.js`、`npm`、`AppImage`、`Codex`、`Claude Code`、`OpenAI`、`Anthropic`、`OpenRouter`、`Databricks`、`Ollama`、模型 ID | 厂商、产品、模型标识原样保留；`PR` 可写作「拉取请求（PR）」 |
| 技术缩写 | `API`、`URL`、`ID`、`UUID`、`UTC`、`IME`、`UI`、`JSON`、`Markdown`、`B` / `KB` / `MB` | 不译；单位与数字之间加半角空格（`{{size}} KB`） |
| 语言端名 | `English`、`简体中文` | 语言选项中永远显示本语言名（`settings.appearance.language.option.*`） |
| 字面排序标签 | `A–Z` | 目录中与英文完全一致（`sidebar.sections.alpha`），不译 |

## 2. 核心名词（目录已统一，必须沿用）

| 英文 | 简体中文 | 规则 / 目录证据 |
|---|---|---|
| community | 社区 | 全局一致；`社区 URL`、`加入社区`、`更换社区` |
| channel | 频道 | 不用「通道」。`频道设置`、`新建频道`、`{{count}} 个频道` |
| direct message / DM | 私信 | `Direct messages → 私信`；空间受限时直接用「私信」，不保留 DM |
| thread | 讨论串 | 见 §6 待审（目录里还残留「话题」「线程」） |
| forum | 论坛 | `Create forum → 创建论坛`、`All forums → 全部论坛` |
| message | 消息 | `发送消息`、`新消息`、`{{count}} 条新消息` |
| relay | 中继 / 中继服务器 | 短标签「中继」（`中继 URL`、`无法重新连接中继。`）；首次出现或解释性文案用「中继服务器」 |
| agent | 智能体 | 产品文案统一「智能体」（`添加智能体`、`{{count}} 个智能体`）；代码与协议标识保持 `agent` |
| harness | 运行器 | `默认运行器`、`选择运行器`。**待审**：与上游产品术语可能冲突，需母语 reviewer 确认 |
| workflow | 工作流 | `{{count}} 个工作流` |
| repository / repo | 代码仓库 / 仓库 | 解释文案「代码仓库」（`添加代码仓库`），短标签「仓库」 |
| pull request | 拉取请求 | 首次出现可标注（PR） |
| mention | 提及 | 动词「提及」（`提及成员`）；计数用「条提及」 |
| reaction | 回应 / 表情回应 | `添加回应`、`打开表情回应`；不要写「反应」 |
| pin / pinned | 置顶 / 已置顶 | star（收藏）是另一动作：`收藏频道` / `已收藏` |
| mute | 静音 | `静音频道`、`取消静音频道` |
| archive | 归档 | `归档频道`、`已归档`、`取消归档频道` |
| draft | 草稿 | `空草稿`、`暂无草稿`、`删除草稿` |
| canvas | 画布 | `画布内容`、`正在加载画布…` |
| section | 分组 | `创建分组`、`重命名分组`、`分组名称` |
| template | 模板 | `新建工作流`/`模板`，不写「模版」 |
| project | 项目 | `添加项目`、`项目已删除` |
| task | 任务 | 项目内的 issue 称「任务」 |
| member | 成员 | `频道成员`、`添加成员`、`{{count}} 名成员` |
| invite | 邀请 | `兑换邀请`、`邀请链接`；redeem = 「兑换」 |
| permission | 权限 | `保存访问权限`、`管理智能体访问权限` |
| role | 角色 | `更改角色`、`管理员`、`访客`、`所有者` |
| bot | 机器人 | `快速添加机器人`、`{{count}} 个托管机器人` |
| model | 模型 | `选择你的模型设置` |
| provider | 服务商 | `正在加载服务商`；不写「提供商」 |
| identity / key | 身份 / 密钥 | `身份密钥`、`私钥`、`导入密钥`；公钥显示为「公开身份」+ `npub` |
| backup | 备份 | `备份密码`、`验证备份`、`重新下载备份`；危险操作必须写明后果 |
| profile | 个人资料 | 见 §6 待审（目录中混用「资料」） |
| notification | 通知 | `{{count}} 条未读通知` |
| settings | 设置 | `频道设置`、`继续设置` |
| appearance | 外观 | `Settings → Appearance` |
| language | 语言 | `界面语言` |
| activity | 动态 | `查看动态`；`Pulse` 功能同译「动态」 |
| composer | 输入框 | `频道消息与输入框` |
| attachment | 附件 | `待上传附件`、`{{count}} 个附件` |
| voice note | 语音消息 | 不写「语音备忘录」 |
| spoiler | 剧透 | `移除剧透标记` |
| inbox | 收件箱 | — |
| team | 团队 | `{{count}} 个团队` |

## 3. 高频动作（按钮优先动词，不加句号）

| 英文 | 简体中文 | 英文 | 简体中文 |
|---|---|---|---|
| Add | 添加 | Move up / down | 上移 / 下移 |
| Apply | 应用 | Open | 打开 |
| Back | 返回 | Redeem | 兑换 |
| Cancel | 取消 | Remove | 移除 |
| Change | 更改 | Rename | 重命名 |
| Close | 关闭 | Reveal | 显示 |
| Collapse | 折叠 | Retry | 重试 |
| Copy | 复制 | Revert | 还原 |
| Create | 创建（新对象用「新建」） | Save | 保存 |
| Delete | 删除 | Search | 搜索 |
| Details | 详情 | Send | 发送 |
| Discard | 丢弃 | Show / Hide | 显示 / 隐藏 |
| Dismiss | 关闭 | Sign in | 登录 |
| Download | 下载 | Sign out | 退出登录 |
| Edit | 编辑 | Skip | 跳过 |
| Expand | 展开 | Start | 启动 |
| Hide | 隐藏 | Stop | 停止 |
| Import | 导入 | Star | 收藏 |
| Insert | 插入 | Try again | 请重试 |
| Install | 安装 | Unarchive | 取消归档 |
| Invite | 邀请 | Unfollow | 取消关注 |
| Join | 加入 | Unlink | 取消链接 |
| Leave | 离开 | Unmute | 取消静音 |
| Manage | 管理 | Unstar | 取消收藏 |
| Mark read | 标记为已读 | Upload | 上传 |
| Mark unread | 标记为未读 | View | 查看 |
| Reply | 回复 | Undo | 撤销 |

`sign out` 一律「退出登录」，不得用「注销」（避免被理解为删号）。

## 4. 状态与语法形态

| 英文形态 | 中文形态 | 目录证据 |
|---|---|---|
| `X...` / `Xing…`（进行中） | 「正在X…」，用全角省略号 | `Adding... → 正在添加…`、`Deleting... → 正在删除…`、`Saving... → 正在保存…` |
| 过去分词 / 完成态 | 「已X」 | `Copied → 已复制`、`Deleted → 已删除`、`Archived → 已归档`、`Connected → 已连接`、`Joined → 已加入`、`Starred → 已收藏` |
| 未达成 / 缺失 | 「未X」或「暂无X」 | `Unread → 未读`、`No drafts → 暂无草稿`、`Not installed → 未安装` |
| 运行态 | 「运行中」/「正在工作」 | `Running → 运行中`、`Working → 工作中`、`agents working → 正在工作的智能体` |
| 空状态 | 「暂无X」/「还没有X」 | `暂无消息`、`还没有工作流` |
| 计数 | `{{count}} ` + 量词 + 名词，数字与量词之间加空格 | 个（频道/智能体/成员/团队/工作流/附件）、条（消息/提及/通知）、名 / 位（成员）、人（参与者） |
| 相对时间 | `{{count}} 天前 / 小时前 / 分钟前 / 周前` | 目录统一，数字与「天」之间加空格 |
| 系统时间词 | `Today → 今天`、`Yesterday → 昨天`、`just now → 刚刚` | 日期分段逻辑不随展示改变（PR5） |
| 复数 | zh-Hans 只有一个形态，`_one` / `_other` 键值相同即可，但必须保留 `{{count}}` | NFR-002 键与变量一致性 |

## 5. 标点与排版（由目录统计得出，PR6 起强制执行）

- 中文整句结尾用 `。`，**不用**英文句点：目录中 233 条以 `。` 结尾，0 条以 `.` 结尾。
- 按钮、菜单项、短标签不加句末标点。
- 省略/进行态用全角 `…`（目录 47 条），不用 ASCII `...`（0 条）。英文源保留 `...`，中文统一 `…`。
- 中文与拉丁字母、数字、插值变量之间加一个半角空格：`频道 ID`、`复制 npub`、`{{count}} 个智能体`。
  现状 144 条含空格 / 115 条未加，审校时按本规则补齐。
- 句中停顿用 `，`（67 条），并列用 `、`（6 条），标签冒号用 `：`（20 条），括号用全角 `（）`（13 条）。
- 引号用 `“”`（如 `“{{query}}”`），不使用直角引号。
- 菜单路径与并列信息沿用中点：`{{view}} · {{scope}}`。
- 语气：自然、简洁、直接；不做逐词直译；危险操作必须点明对象与后果。

## 6. 待审冲突（审校时必须统一，不得各表面自行选择）

| 冲突 | 现状（目录） | 冻结结论 |
|---|---|---|
| thread | `讨论串`（`Thread`、`Expand thread`）/ `话题`（`Collapse thread`、`Follow thread`、`Unfollow thread`）/ `线程`（`Thread deleted`） | 统一 `讨论串`；`线程` 属系统线程歧义，禁止用于 thread |
| profile | `个人资料`（`Saving profile`）/ `资料`（`Your profile`） | 统一 `个人资料`，短标签可用 `资料` 但同一面板内不得混用 |
| All channels | `所有频道` / `全部频道` | 统一 `全部频道`（与 `全部论坛`、`全部停止` 一致） |
| Mark unread | `标为未读` / `标记为未读` | 统一 `标记为未读` |
| Name | `名称`（对象名）/ `姓名`（人） | 按语义固定：人 = `姓名`，对象 = `名称` |
| Open | `公开`（可见性）与 `打开`（动作） | 可见性 `公开`，动作 `打开`；两者不得互相覆盖 |
| Pulse / Activity | 都译 `动态` | 允许；新增功能名时须先改本表 |
| harness | `运行器` | 保留，需母语 reviewer 终确认（§8 风险项） |

## 7. 禁译清单（FR-010）

以下内容永不进入翻译目录、永不改写：用户消息与用户名/频道名/项目名、agent 与
harness 输出的自由文本、文件名与路径、代码与 diff、终端输出、Nostr 事件内容与
relay payload 字段、事件 kind、协议字段名、模型名与 ID、URL、`npub` / `nsec`、
服务端原始错误详情（未知错误保留原文并可复制；只有稳定错误码可映射中文摘要）。
`PR6` 静态门禁把这类字符串记为 `user-content` / `protocol-field` / `code-sample` /
`test-fixture` / `brand-name` / `class-name` / `url` / `non-ui-identifier`（见
`desktop/scripts/hardcoded-strings-baseline.json`）。

## 8. 变更记录

- PR6（本文件）：从 `zh-Hans.json` 1160 条现值提取冻结表；新增标点/排版量化规则；
  登记 §6 的 7 组冲突。术语调整须由母语 reviewer 签字后，一次性更新目录与本表。
