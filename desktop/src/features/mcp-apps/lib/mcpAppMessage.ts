import type { McpAppMessage } from "@/features/mcp-apps/lib/mcpAppBridge";

export const MCP_APP_POST_MAX_CHARS = 8_000;

function normalizeText(value: string): string {
  return value.trim().replace(/\n(?:[ \t]*\n){2,}/g, "\n\n");
}

export function mcpAppMessageText(message: McpAppMessage): string | null {
  const blocks = Array.isArray(message.content)
    ? message.content
    : [message.content];
  const text = blocks
    .flatMap((block) => {
      if (typeof block === "string") return [block];
      if (
        block &&
        typeof block === "object" &&
        !Array.isArray(block) &&
        (block as Record<string, unknown>).type === "text" &&
        typeof (block as Record<string, unknown>).text === "string"
      ) {
        return [(block as Record<string, unknown>).text as string];
      }
      return [];
    })
    .map(normalizeText)
    .filter(Boolean)
    .join("\n\n");
  return text || null;
}
