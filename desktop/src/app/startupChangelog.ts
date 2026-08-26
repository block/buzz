export type StartupChangelogEntry = {
  date: string;
  items: string[];
};

const CHANGELOG_WINDOW_DAYS = 10;
const DAY_MS = 24 * 60 * 60 * 1000;

export const STARTUP_CHANGELOG: StartupChangelogEntry[] = [
  {
    date: "2026-08-26",
    items: [
      "【主要更新】支持通过 SSH 连接其他电脑上的 Codex task，并可直接从 SSH config 选择连接配置。",
      "修复 SSH task 被默认本地 task 覆盖的问题，创建 Agent 时保持远程任务选择。",
      "修复迁移后 Agent 重复的问题，并在恢复身份时保留 Codex task 绑定。",
      "Codex Agent 可以读取更长的历史记录，并保持同一 task 只对应一个有效实例。",
      "更新日志改为按日期展示，并仅保留最新日期往前 10 个自然日。",
      "修复手动安装时旧 Buzz 进程未退出、导致主程序没有被新版本覆盖的问题。",
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
