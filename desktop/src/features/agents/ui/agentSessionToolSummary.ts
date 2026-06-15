import type { ToolStatus, TranscriptItem } from "./agentSessionTypes";
import {
  getBuzzToolInfo,
  normalizeToolNameText,
} from "./agentSessionToolCatalog";
import { asRecord, getToolString } from "./agentSessionUtils";

export type CompactToolKind =
  | "shell"
  | "read_file"
  | "view_image"
  | "str_replace"
  | "todo"
  | "stop_hook"
  | "post_compact_hook"
  | "dev_mcp";

export type CompactToolSummary = {
  kind: CompactToolKind;
  label: string;
  preview: string | null;
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

/** Whether this tool row should use the muted compact developer summary. */
export function isCompactDeveloperTool(item: ToolItem): boolean {
  if (item.buzzToolName && getBuzzToolInfo(item.buzzToolName)) {
    return false;
  }
  return resolveDeveloperToolKind(item) !== null;
}

/** Build the compact summary label and preview for developer MCP tool rows. */
export function buildCompactToolSummary(item: ToolItem): CompactToolSummary {
  const kind = resolveDeveloperToolKind(item) ?? "dev_mcp";
  const preview = extractCompactToolPreview(item, kind);
  return {
    kind,
    label: compactToolLabel(kind, item.status, item.isError),
    preview,
  };
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
    return base === "shell" ? "shell" : "dev_mcp";
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
  status: ToolStatus,
  isError: boolean,
): string {
  const failed = isError || status === "failed";
  const running = status === "executing" || status === "pending";

  const labels: Record<
    CompactToolKind,
    { completed: string; running: string; failed: string }
  > = {
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

  const set = labels[kind];
  if (failed) return set.failed;
  if (running) return set.running;
  return set.completed;
}

function extractCompactToolPreview(
  item: ToolItem,
  kind: CompactToolKind,
): string | null {
  const args = item.args;

  switch (kind) {
    case "shell":
      return getToolString(args, ["command"]);
    case "read_file":
    case "str_replace":
      return getToolString(args, ["path"]);
    case "view_image":
      return getToolString(args, ["source"]);
    case "todo":
      return getTodoPreview(args);
    case "stop_hook":
    case "post_compact_hook":
      return null;
    case "dev_mcp":
      return (
        getToolString(args, ["command", "path", "source", "query", "name"]) ??
        null
      );
  }
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
