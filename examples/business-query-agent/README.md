# 经营查询助手 Persona

此 Persona 只能与 `BUSINESS_AGENT_READ_ENABLED=true` 的 `buzz-acp` 一起部署。
Host 会忽略普通 `BUZZ_ACP_MCP_COMMAND`，为每个真实 Buzz 用户事件签发独立
Delegation，并只注入 `business-read-mcp`。Heartbeat 必须保持关闭。

本目录不包含密钥、Delegation 或 MCP 静态配置。运行配置见
`docs/business-agent/operations.md`。
