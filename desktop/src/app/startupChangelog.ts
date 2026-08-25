export type StartupChangelogEntry = {
  title: string;
  items: string[];
};

export const STARTUP_CHANGELOG: StartupChangelogEntry[] = [
  {
    title: "Agent handoff",
    items: [
      "Agent 可以根据当前 ACP 会话自动生成 Markdown 交接内容。",
      "发送交接时会自动通知目标 Agent，并保留加密交接记录。",
      "交接候选列表只显示当前频道中的未删除 Agent。",
      "交接记录会显示 Agent 所属用户，便于区分同名或已删除的 Agent。",
    ],
  },
  {
    title: "在线更新",
    items: [
      "Buzz Codex Lab 现在支持检查、下载并安装经过签名验证的更新。",
      "设置页和侧边栏会提示可用的新版本。",
    ],
  },
  {
    title: "Codex 与连接",
    items: [
      "增强 Windows 上 Codex shared runtime 的自动启动与诊断能力。",
      "支持通过局域网别名连接，并改进 Codex task runtime 的发现速度。",
    ],
  },
  {
    title: "Codex Lab 修复",
    items: [
      "恢复文件与媒体访问能力。",
      "修复邀请链接在 Buzz Codex Lab 中无法正常打开的问题。",
    ],
  },
  {
    title: "构建体验",
    items: ["Windows 安装包构建现在会复用 Cargo 增量缓存，减少重复编译时间。"],
  },
];
