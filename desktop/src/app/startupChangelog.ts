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
    ],
  },
  {
    title: "构建体验",
    items: ["Windows 安装包构建现在会复用 Cargo 增量缓存，减少重复编译时间。"],
  },
];
