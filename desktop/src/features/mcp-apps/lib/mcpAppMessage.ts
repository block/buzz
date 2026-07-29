import type { McpAppMessage } from "@/features/mcp-apps/lib/mcpAppBridge";

export const MCP_APP_POST_MAX_CHARS = 8_000;
export const MCP_APP_POST_MAX_LINES = 120;
const MCP_APP_TITLE_MAX_CHARS = 80;

function isUnsafeDisplayCharacter(character: string): boolean {
  const codePoint = character.codePointAt(0);
  if (codePoint === undefined) return false;
  return (
    (codePoint <= 0x1f && codePoint !== 0x09 && codePoint !== 0x0a) ||
    (codePoint >= 0x7f && codePoint <= 0x9f) ||
    codePoint === 0x061c ||
    codePoint === 0x200b ||
    codePoint === 0x200e ||
    codePoint === 0x200f ||
    (codePoint >= 0x202a && codePoint <= 0x202e) ||
    codePoint === 0x2060 ||
    (codePoint >= 0x2066 && codePoint <= 0x2069) ||
    codePoint === 0xfeff
  );
}

function normalizeText(value: string): string {
  return Array.from(
    value.normalize("NFC").replace(/\r\n?|\u2028|\u2029/g, "\n"),
  )
    .filter((character) => !isUnsafeDisplayCharacter(character))
    .join("")
    .trim()
    .replace(/\n(?:[ \t]*\n){2,}/g, "\n\n");
}

export function mcpAppDisplayLabel(
  value: string,
  fallback: string,
  maxChars = MCP_APP_TITLE_MAX_CHARS,
): string {
  return (
    Array.from(normalizeText(value).replace(/\s+/g, " "))
      .slice(0, maxChars)
      .join("")
      .trim() || fallback
  );
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
  const title = mcpAppDisplayLabel(appTitle, "Channel app");
  return `MCP App · ${title}\n\n${content}`;
}
