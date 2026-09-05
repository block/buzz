export type StartupChangelogEntry = {
  date: string;
  items: string[];
};

const CHANGELOG_WINDOW_DAYS = 10;
const DAY_MS = 24 * 60 * 60 * 1000;

export const STARTUP_CHANGELOG: StartupChangelogEntry[] = [
  {
    date: "2026-09-05",
    items: [
      "【主要更新】兼容新版 Codex Desktop 的版本化 backend 路径，连接 task 前可以正确检测并接管已有私有 app-server。",
      "修复 Buzz 退出后 shared Codex runtime 锁住 Codex 更新目录的问题，并恢复 Windows Agent 停止与清理行为。",
      "Codex Code Mode host 未安装时自动降级，不再输出误导性的运行时警告。",
    ],
  },
  {
    date: "2026-09-02",
    items: [
      "修复登录或重连后 Inbox 线程根消息、父消息及部分上下文无法加载的问题。",
      "桌面端会保留提前到达的 Relay 认证 challenge，降低登录阶段的连接竞态。",
      "移动端支持通过配对继承 LAN Relay，并改善内网媒体上传、预览和连接切换。",
    ],
  },
  {
    date: "2026-08-30",
    items: [
      "【主要更新】修复内网 LAN Relay 认证协议不匹配：内网连接现在可以正常完成 NIP-42 登录。",
      "同一个 Community 支持 LAN 优先、不可用时自动回退公网，并可通过按钮重新检测并切回 LAN。",
      "修复“内网不可用”提示过于笼统的问题，现在会显示连接、认证和公网回退的具体结果。",
    ],
  },
  {
    date: "2026-08-28",
    items: [
      "修复旧 Community 与 WebView 状态可能导致 Buzz 启动后一直加载的问题。",
      "修复公网 Community 的 HTTP 请求错误回退到 LAN Host、导致频道不可用的问题。",
    ],
  },
  {
    date: "2026-08-27",
    items: [
      "诊断版本：撤回 182519d9 的连接相关改动，同时恢复 LAN HTTP fallback，用于验证断连问题来源。",
      "加入 Community 时公网 Relay 地址改为可选，仅填写 LAN Relay 地址也可以完成连接。",
      "修复公网和内网 WebSocket 长连接心跳处理，降低空闲连接周期性断开的概率。",
      "修复连接写入失败后原生连接任务退出但前端未收到错误，恢复正常自动重连。",
      "网络断开时仍可退出当前 Community，不再被卡在无法访问的社区。",
      "加入 Community 时 join-policy 请求失败不再阻塞 WebSocket 加入。",
      "修复公网 WebSocket 空闲时被本地连接监测误判为断线的问题，减少穿透环境下的周期性掉线。",
      "加入 Community 时支持填写可选的 LAN relay URL，并在连接状态中显示当前使用 LAN 或公网 relay。",
      "连接状态卡片增加刷新按钮，切换网络环境后可以立即重新检测连接路径。",
    ],
  },
  {
    date: "2026-08-26",
    items: [
      "【主要更新】支持通过 SSH 连接其他电脑上的 Codex task，并可直接从 SSH config 选择连接配置。",
      "修复 SSH task 被默认本地 task 覆盖的问题，创建 Agent 时保持远程任务选择。",
      "修复迁移后 Agent 重复的问题，并在恢复身份时保留 Codex task 绑定。",
      "Codex Agent 可以读取更长的历史记录，并保持同一 task 只对应一个有效实例。",
      "更新日志改为按日期展示，并仅保留最新日期往前 10 个自然日。",
      "修复手动安装时旧 Buzz 进程未退出、导致主程序没有被新版本覆盖的问题。",
      "重新发布 Buzz Codex Lab v0.5.15，替换此前未正确覆盖主程序的安装包。",
      "修复同一 Codex task 通过多个 relay 地址重复启动，导致 Activity、回复和思考状态重复的问题。",
    ],
  },
  {
    date: "2026-08-25",
    items: [
      "增加 Buzz Codex Lab 签名在线更新和本地发布能力。",
      "Windows 改为直接启动 Codex shared runtime，并增强启动诊断。",
      "断线重连会按频道无损补回遗漏消息。",
      "恢复 Codex Lab 的媒体访问和邀请链接。",
    ],
  },
  {
    date: "2026-08-24",
    items: [
      "增加 Agent handoff：由来源 Agent 自动总结并生成 Markdown 交接内容。",
      "handoff 支持跨用户 Agent，并显示 Agent 所属用户和删除状态。",
      "handoff 候选仅显示当前频道内仍存在的 Agent。",
      "增强 Windows Codex runtime 自动启动，并支持局域网别名。",
    ],
  },
  {
    date: "2026-08-23",
    items: ["加快本地 Codex task 的发现速度。"],
  },
  {
    date: "2026-08-22",
    items: [
      "增加 Agent handoff 图形界面。",
      "Activity 中可以随时查看 Agent handoff 历史。",
    ],
  },
  {
    date: "2026-08-21",
    items: [
      "Activity 独立显示 Agent 运行状态，不再只在生成内容时可见。",
      "其他参与者可以查看 Agent 思考过程并选择性停止 Agent 输出。",
      "修复跨用户 Agent mention，并隐藏未就绪的共享身份。",
      "稳定 macOS Codex task runtime，安装包离线包含 ACP 资源。",
    ],
  },
  {
    date: "2026-08-20",
    items: [
      "Agent 可以单独配置是否接受非所有者私聊指令。",
      "Agent 访问策略会同步到关联实例和远程启动环境。",
      "允许在没有 LLM Provider 凭据时单独编辑访问权限。",
      "修复同名 Agent mention 和重连消息丢失。",
    ],
  },
  {
    date: "2026-08-19",
    items: [
      "支持查看绑定 Codex task 的历史记录。",
      "Agent 之间可以传递文件。",
      "增加停止 Agent 输出按钮，并可选择停止指定 Agent。",
      "修复 Agent mention、公式渲染和 Agent 编辑。",
      "MCP 文件路径支持 `~` 和 MSYS 用户目录。",
    ],
  },
  {
    date: "2026-08-18",
    items: [
      "Markdown、文本、代码、JSON、CSV 和 PDF 支持应用内预览。",
      "支持上传和下载通用文件，并保留原始文件名。",
      "修复 thread 消息和 Markdown 文件发送。",
      "支持竖屏视频，并自动压缩过大的 Agent 分享头像。",
      "安装更新前会停止 Buzz 相关进程。",
    ],
  },
  {
    date: "2026-08-17",
    items: [
      "修复用户与 Agent 之间的消息和文件传递。",
      "文件链接可以在 Buzz 中直接下载。",
      "开发 MCP 支持经过认证的附件下载。",
    ],
  },
];

function utcDay(date: string) {
  const [year, month, day] = date.split("-").map(Number);
  return Date.UTC(year, month - 1, day);
}

export function recentStartupChangelog(
  entries: StartupChangelogEntry[] = STARTUP_CHANGELOG,
  windowDays = CHANGELOG_WINDOW_DAYS,
) {
  if (entries.length === 0 || windowDays <= 0) return [];
  const newestDay = Math.max(...entries.map((entry) => utcDay(entry.date)));
  const cutoff = newestDay - (windowDays - 1) * DAY_MS;
  return entries
    .filter((entry) => utcDay(entry.date) >= cutoff)
    .sort((left, right) => right.date.localeCompare(left.date));
}

export const RECENT_STARTUP_CHANGELOG = recentStartupChangelog();
