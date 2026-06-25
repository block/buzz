import type { ToolStatus, TranscriptItem } from "./agentSessionTypes";
import {
  formatToolTitle,
  getBuzzToolInfo,
  isGenericToolTitle,
  normalizeToolNameText,
} from "./agentSessionToolCatalog";
import {
  asRecord,
  getToolString,
  getToolStringList,
} from "./agentSessionUtils";

export type CompactToolKind =
  | "shell"
  | "read_file"
  | "view_image"
  | "str_replace"
  | "todo"
  | "stop_hook"
  | "post_compact_hook"
  | "dev_mcp"
  | "buzz"
  | "generic";

export type CompactToolSummary = {
  kind: CompactToolKind;
  label: string;
  preview: string | null;
  /** When set, the compact row renders a tiny image instead of text preview. */
  thumbnailSrc: string | null;
  presentation: "inline" | "message";
};

const DEVELOPER_TOOL_BASES = new Set([
  "shell",
  "read_file",
  "view_image",
  "str_replace",
  "todo",
  "stop",
  "postcompact",
]);

type ToolItem = Extract<TranscriptItem, { type: "tool" }>;

/** Build the muted compact summary label and preview for any tool row. */
export function buildCompactToolSummary(item: ToolItem): CompactToolSummary {
  const kind = resolveCompactToolKind(item);
  const messageSendPreview = extractMessageSendPreview(item, kind);
  if (messageSendPreview !== undefined) {
    return {
      kind,
      label: compactMessageSendLabel(item.status, item.isError),
      preview: messageSendPreview,
      thumbnailSrc: null,
      presentation: "message",
    };
  }

  const { preview, thumbnailSrc } = extractCompactToolPreview(item, kind);
  return {
    kind,
    label: compactToolLabel(kind, item, item.status, item.isError),
    preview,
    thumbnailSrc,
    presentation: "inline",
  };
}

function resolveCompactToolKind(item: ToolItem): CompactToolKind {
  const developerKind = resolveDeveloperToolKind(item);
  if (developerKind) {
    return developerKind;
  }

  for (const value of [item.buzzToolName, item.toolName, item.title]) {
    if (value && getBuzzToolInfo(value)) {
      return "buzz";
    }
  }

  return "generic";
}

function resolveDeveloperToolKind(item: ToolItem): CompactToolKind | null {
  for (const value of [item.toolName, item.title, item.buzzToolName]) {
    const kind = classifyDeveloperToolName(value);
    if (kind) return kind;
  }
  return null;
}

function classifyDeveloperToolName(
  value: string | null | undefined,
): CompactToolKind | null {
  if (!value) return null;

  const normalized = normalizeToolNameText(value);
  const base = stripMcpServerPrefix(normalized);

  if (base === "shell" || normalized.endsWith("_shell")) {
    return "shell";
  }
  if (base === "read_file") return "read_file";
  if (base === "view_image") return "view_image";
  if (base === "str_replace") return "str_replace";
  if (base === "todo") return "todo";
  if (base === "stop") return "stop_hook";
  if (base === "postcompact") return "post_compact_hook";

  if (DEVELOPER_TOOL_BASES.has(base)) {
    return "dev_mcp";
  }

  if (normalized.includes("buzz_dev_mcp")) {
    return "dev_mcp";
  }

  return null;
}

function stripMcpServerPrefix(normalized: string): string {
  return normalized.replace(/^buzz_dev_mcp_/, "");
}

function compactToolLabel(
  kind: CompactToolKind,
  item: ToolItem,
  status: ToolStatus,
  isError: boolean,
): string {
  const failed = isError || status === "failed";
  const running = status === "executing" || status === "pending";

  if (kind === "buzz") {
    const title = formatToolTitle(
      item.buzzToolName ?? item.toolName,
      item.title,
    );
    if (failed) return `${title} failed`;
    if (running) return title;
    return title;
  }

  const labels: Record<
    Exclude<CompactToolKind, "buzz">,
    { completed: string; running: string; failed: string }
  > = {
    generic: {
      completed: "Ran tool",
      running: "Running tool",
      failed: "Tool failed",
    },
    ...developerToolLabels(),
  };

  const set = labels[kind];
  if (failed) return set.failed;
  if (running) return set.running;
  return set.completed;
}

function developerToolLabels(): Record<
  Exclude<CompactToolKind, "buzz" | "generic">,
  { completed: string; running: string; failed: string }
> {
  return {
    shell: {
      completed: "Ran command",
      running: "Running command",
      failed: "Command failed",
    },
    read_file: {
      completed: "Read file",
      running: "Reading file",
      failed: "Read failed",
    },
    view_image: {
      completed: "Viewed image",
      running: "Viewing image",
      failed: "View failed",
    },
    str_replace: {
      completed: "Edited file",
      running: "Editing file",
      failed: "Edit failed",
    },
    todo: {
      completed: "Updated todos",
      running: "Updating todos",
      failed: "Todo update failed",
    },
    stop_hook: {
      completed: "Checked todos",
      running: "Checking todos",
      failed: "Todo check failed",
    },
    post_compact_hook: {
      completed: "Synced todos",
      running: "Syncing todos",
      failed: "Todo sync failed",
    },
    dev_mcp: {
      completed: "Ran tool",
      running: "Running tool",
      failed: "Tool failed",
    },
  };
}

function compactMessageSendLabel(status: ToolStatus, isError: boolean) {
  if (isError || status === "failed") {
    return "Send Message failed";
  }
  return "Send Message";
}

type CompactToolPreview = {
  preview: string | null;
  thumbnailSrc: string | null;
};

function extractCompactToolPreview(
  item: ToolItem,
  kind: CompactToolKind,
): CompactToolPreview {
  const args = item.args;

  switch (kind) {
    case "shell":
      return textPreview(getToolString(args, ["command"]));
    case "read_file":
    case "str_replace":
      return textPreview(getToolString(args, ["path"]));
    case "view_image":
      return getViewImagePreview(getToolString(args, ["source"]));
    case "todo":
      return textPreview(getTodoPreview(args));
    case "stop_hook":
    case "post_compact_hook":
      return emptyPreview();
    case "dev_mcp":
    case "generic":
      return textPreview(
        getToolString(args, [
          "command",
          "path",
          "source",
          "query",
          "name",
          "content",
          "message",
        ]) ??
          (item.title && !isGenericToolTitle(item.title) ? item.title : null),
      );
    case "buzz":
      return textPreview(extractBuzzToolPreview(args));
  }
}

function extractBuzzToolPreview(args: Record<string, unknown>): string | null {
  const content = getToolString(args, ["content", "message", "text", "body"]);
  if (content) {
    return content;
  }

  const query = getToolString(args, ["query", "search"]);
  if (query) {
    return query;
  }

  const channelId = getToolString(args, ["channel_id", "channelId"]);
  if (channelId) {
    return channelId;
  }

  const workflowId = getToolString(args, ["workflow_id", "workflowId"]);
  if (workflowId) {
    return workflowId;
  }

  const pubkeys = getToolStringList(args, ["pubkeys", "pubkey"]);
  if (pubkeys.length === 1) {
    return pubkeys[0];
  }
  if (pubkeys.length > 1) {
    return `${pubkeys.length} users`;
  }

  return getToolString(args, ["event_id", "eventId", "name"]);
}

function extractMessageSendPreview(
  item: ToolItem,
  kind: CompactToolKind,
): string | null | undefined {
  if (isBuzzSendMessageTool(item)) {
    return extractBuzzToolMessageContent(item.args);
  }

  if (kind !== "shell") {
    return undefined;
  }

  const command = getToolString(item.args, ["command"]);
  return command ? extractBuzzCliSendMessageContent(command) : undefined;
}

function isBuzzSendMessageTool(item: ToolItem) {
  return [item.buzzToolName, item.toolName, item.title].some((value) => {
    if (!value) return false;
    return normalizeToolNameText(value) === "send_message";
  });
}

function extractBuzzToolMessageContent(
  args: Record<string, unknown>,
): string | null {
  return getToolString(args, ["content", "message", "text", "body"]);
}

function extractBuzzCliSendMessageContent(
  command: string,
): string | null | undefined {
  const tokens = tokenizeShellCommand(command);
  const commandRange = findBuzzMessagesSendCommand(tokens);
  if (!commandRange) {
    return undefined;
  }

  const content = getFlagValue(tokens, commandRange.sendIndex + 1, "--content");
  if (!content) {
    return null;
  }
  if (content !== "-") {
    return content;
  }

  const pipedContent = extractSimpleEchoPipeContent(
    tokens,
    commandRange.buzzIndex,
  );
  return pipedContent ?? null;
}

function tokenizeShellCommand(command: string): string[] {
  const tokens: string[] = [];
  let current = "";
  let quote: "'" | '"' | null = null;
  let escaping = false;

  const pushCurrent = () => {
    if (current.length > 0) {
      tokens.push(current);
      current = "";
    }
  };

  for (const char of command) {
    if (escaping) {
      current += char;
      escaping = false;
      continue;
    }

    if (char === "\\" && quote !== "'") {
      escaping = true;
      continue;
    }

    if (quote) {
      if (char === quote) {
        quote = null;
      } else {
        current += char;
      }
      continue;
    }

    if (char === "'" || char === '"') {
      quote = char;
      continue;
    }

    if (/\s/.test(char)) {
      pushCurrent();
      continue;
    }

    if (char === "|" || char === ";" || char === "&") {
      pushCurrent();
      tokens.push(char);
      continue;
    }

    current += char;
  }

  if (escaping) {
    current += "\\";
  }
  pushCurrent();
  return tokens;
}

function findBuzzMessagesSendCommand(
  tokens: string[],
): { buzzIndex: number; sendIndex: number } | null {
  for (let i = 0; i < tokens.length; i++) {
    if (!isBuzzExecutable(tokens[i])) {
      continue;
    }

    const messagesIndex = tokens.indexOf("messages", i + 1);
    if (messagesIndex === -1) {
      continue;
    }
    if (
      messagesIndex > i &&
      hasCommandSeparator(tokens, i + 1, messagesIndex)
    ) {
      continue;
    }
    if (tokens[messagesIndex + 1] === "send") {
      return { buzzIndex: i, sendIndex: messagesIndex + 1 };
    }
  }

  return null;
}

function isBuzzExecutable(token: string) {
  return token === "buzz" || token.split(/[\\/]/).pop() === "buzz";
}

function hasCommandSeparator(tokens: string[], start: number, end: number) {
  for (let i = start; i < end; i++) {
    if (isCommandSeparator(tokens[i])) {
      return true;
    }
  }
  return false;
}

function isCommandSeparator(token: string) {
  return token === "|" || token === ";" || token === "&";
}

function getFlagValue(tokens: string[], start: number, flag: string) {
  for (let i = start; i < tokens.length; i++) {
    const token = tokens[i];
    if (isCommandSeparator(token)) {
      return null;
    }
    if (token === flag) {
      return tokens[i + 1] && !isCommandSeparator(tokens[i + 1])
        ? tokens[i + 1]
        : null;
    }
    if (token.startsWith(`${flag}=`)) {
      return token.slice(flag.length + 1);
    }
  }
  return null;
}

function extractSimpleEchoPipeContent(
  tokens: string[],
  buzzIndex: number,
): string | null {
  const pipeIndex = tokens.lastIndexOf("|", buzzIndex);
  if (pipeIndex <= 0) {
    return null;
  }

  const echoStart = findSegmentStart(tokens, pipeIndex - 1);
  const leftSegment = tokens.slice(echoStart, pipeIndex);
  if (leftSegment[0] !== "echo") {
    return null;
  }

  const contentTokens = leftSegment
    .slice(1)
    .filter((token) => !token.startsWith("-"));
  return contentTokens.length > 0 ? contentTokens.join(" ") : null;
}

function findSegmentStart(tokens: string[], beforeIndex: number) {
  for (let i = beforeIndex; i >= 0; i--) {
    if (isCommandSeparator(tokens[i])) {
      return i + 1;
    }
  }
  return 0;
}

function textPreview(preview: string | null): CompactToolPreview {
  return { preview, thumbnailSrc: null };
}

function emptyPreview(): CompactToolPreview {
  return { preview: null, thumbnailSrc: null };
}

function getViewImagePreview(source: string | null): CompactToolPreview {
  if (!source) {
    return emptyPreview();
  }

  const trimmed = source.trim();
  if (
    trimmed.startsWith("data:image/") ||
    trimmed.startsWith("http://") ||
    trimmed.startsWith("https://")
  ) {
    return {
      preview: trimmed,
      thumbnailSrc: trimmed,
    };
  }

  const basename = trimmed.split(/[/\\]/).pop() ?? trimmed;
  return {
    preview: basename,
    thumbnailSrc: null,
  };
}

function getTodoPreview(args: Record<string, unknown>): string | null {
  const todos = args.todos;
  if (!Array.isArray(todos)) {
    return "todo list";
  }
  if (todos.length === 0) {
    return "empty list";
  }

  const first = todos[0];
  const firstText =
    first && typeof first === "object"
      ? getToolString(asRecord(first), ["text"])
      : null;

  if (firstText) {
    return todos.length > 1 ? `${firstText} (+${todos.length - 1})` : firstText;
  }

  return `${todos.length} item${todos.length === 1 ? "" : "s"}`;
}
