import type { McpAppMessage } from "@/features/mcp-apps/lib/mcpAppBridge";

export const MCP_APP_POST_MAX_CHARS = 8_000;
const MCP_APP_TITLE_MAX_CHARS = 80;

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

export function mcpAppAttributedMessage(
  appTitle: string,
  content: string,
): string {
  const title =
    Array.from(appTitle, (character) => {
      const codePoint = character.codePointAt(0) ?? 0;
      return codePoint <= 0x1f || codePoint === 0x7f ? " " : character;
    })
      .join("")
      .replace(/\s+/g, " ")
      .trim()
      .slice(0, MCP_APP_TITLE_MAX_CHARS) || "Channel app";
  return `MCP App · ${title}\n\n${content}`;
}
