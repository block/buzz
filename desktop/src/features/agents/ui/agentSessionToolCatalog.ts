import {
  CheckCircle2,
  CircleDot,
  Clock3,
  Hash,
  MessageSquare,
  Search,
  Send,
  Users,
  Workflow,
  XCircle,
} from "lucide-react";

import type { SproutToolInfo, ToolStatus } from "./agentSessionTypes";

export function normalizeToolStatus(status: string): ToolStatus {
  const normalized = status.toLowerCase();
  if (
    normalized.includes("complete") ||
    normalized.includes("success") ||
    normalized === "done"
  ) {
    return "completed";
  }
  if (normalized.includes("fail") || normalized.includes("error")) {
    return "failed";
  }
  if (normalized.includes("pending")) {
    return "pending";
  }
  return "executing";
}

export function getToolStatusDisplay(status: ToolStatus, isError: boolean) {
  if (isError || status === "failed") {
    return {
      label: "Error",
      Icon: XCircle,
      state: "output-error" as const,
      variant: "destructive" as const,
    };
  }
  if (status === "completed") {
    return {
      label: "Done",
      Icon: CheckCircle2,
      state: "output-available" as const,
      variant: "secondary" as const,
    };
  }
  if (status === "pending") {
    return {
      label: "Pending",
      Icon: CircleDot,
      state: "input-streaming" as const,
      variant: "secondary" as const,
    };
  }
  return {
    label: "Running",
    Icon: Clock3,
    state: "input-available" as const,
    variant: "secondary" as const,
  };
}

const SPROUT_READ_TOOLS = new Set([
  "get_messages",
  "get_channel_history",
  "get_thread",
  "search",
  "get_feed",
  "get_reactions",
  "list_channels",
  "get_channel",
  "get_users",
  "get_presence",
  "list_channel_members",
  "list_dms",
  "get_canvas",
  "list_workflows",
  "get_workflow_runs",
  "get_event",
  "get_user_notes",
  "get_contact_list",
]);

const SPROUT_WRITE_TOOLS = new Set([
  "send_message",
  "send_diff_message",
  "edit_message",
  "delete_message",
  "add_reaction",
  "remove_reaction",
  "join_channel",
  "leave_channel",
  "update_channel",
  "set_channel_topic",
  "set_channel_purpose",
  "open_dm",
  "set_profile",
  "set_presence",
  "trigger_workflow",
  "approve_step",
  "create_channel",
  "archive_channel",
  "unarchive_channel",
  "add_channel_member",
  "remove_channel_member",
  "add_dm_member",
  "hide_dm",
  "set_canvas",
  "create_workflow",
  "update_workflow",
  "delete_workflow",
  "set_channel_add_policy",
  "vote_on_post",
  "publish_note",
  "set_contact_list",
]);

const SPROUT_TOOL_NAMES = new Set([
  ...SPROUT_READ_TOOLS,
  ...SPROUT_WRITE_TOOLS,
]);

const SPROUT_TOOL_NAMES_BY_LENGTH = [...SPROUT_TOOL_NAMES].sort(
  (left, right) => right.length - left.length,
);

const SPROUT_TOOL_TITLE_ALIASES: Array<[RegExp, string]> = [
  [/\bsending message to channel\b/, "send_message"],
  [/\bretrieving recent messages from channel\b/, "get_messages"],
  [/\bgetting channel details\b/, "get_channel"],
  [/\bgetting user information\b/, "get_users"],
  [/\bsearching relay history\b/, "search"],
  [/\bgetting thread\b/, "get_thread"],
  [/\badding reaction\b/, "add_reaction"],
  [/\bremoving reaction\b/, "remove_reaction"],
];

export function getSproutToolInfo(title: string): SproutToolInfo | null {
  const name = normalizeToolName(title);
  const isRead = SPROUT_READ_TOOLS.has(name);
  const isWrite = SPROUT_WRITE_TOOLS.has(name);
  if (!isRead && !isWrite) {
    return null;
  }

  if (name.includes("workflow") || name === "approve_step") {
    return {
      icon: Workflow,
      label: isRead
        ? "Reads workflow state from Sprout."
        : "Updates workflow state in Sprout.",
      tone: isWrite ? "write" : "read",
    };
  }
  if (
    name.includes("channel") ||
    name.includes("messages") ||
    name === "get_thread"
  ) {
    return {
      icon: Hash,
      label: isRead
        ? "Reads channel context from the Sprout relay."
        : "Changes channel state in the Sprout relay.",
      tone: isWrite ? "write" : "read",
    };
  }
  if (
    name.includes("user") ||
    name.includes("member") ||
    name.includes("presence")
  ) {
    return {
      icon: Users,
      label: isRead
        ? "Reads Sprout identity or presence data."
        : "Updates Sprout identity or membership data.",
      tone: isWrite ? "write" : "admin",
    };
  }
  if (name.includes("search") || name === "get_feed") {
    return {
      icon: Search,
      label: "Searches relay-visible Sprout history.",
      tone: "read",
    };
  }
  if (
    name.startsWith("send_") ||
    name.includes("reaction") ||
    name === "publish_note"
  ) {
    return {
      icon: Send,
      label: "Publishes relay-visible Sprout activity.",
      tone: "write",
    };
  }

  return {
    icon: MessageSquare,
    label: isRead ? "Reads from Sprout." : "Writes to Sprout.",
    tone: isWrite ? "write" : "read",
  };
}

export function normalizeToolName(title: string): string {
  const knownName = findSproutToolName(title, true);
  if (knownName) return knownName;

  const normalized = normalizeToolNameText(title).replace(/^sprout_/, "");
  return normalized.match(/[a-z][a-z0-9_]+/)?.[0] ?? normalized;
}

export function normalizeToolNameText(value: string): string {
  return value
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9_]+/g, "_")
    .replace(/_+/g, "_")
    .replace(/^_+|_+$/g, "");
}

export function findSproutToolName(value: string, includeShortNames: boolean) {
  const alias = findSproutToolAlias(value);
  if (alias) return alias;

  const normalized = normalizeToolNameText(value);
  return (
    SPROUT_TOOL_NAMES_BY_LENGTH.find(
      (name) =>
        (includeShortNames || name.length >= 8) && normalized.includes(name),
    ) ?? null
  );
}

function findSproutToolAlias(value: string) {
  const normalizedPhrase = value
    .trim()
    .toLowerCase()
    .replace(/[_-]+/g, " ")
    .replace(/\s+/g, " ");
  return (
    SPROUT_TOOL_TITLE_ALIASES.find(([pattern]) =>
      pattern.test(normalizedPhrase),
    )?.[1] ?? null
  );
}

export function isGenericToolTitle(value: string): boolean {
  const normalized = normalizeToolNameText(value);
  return (
    normalized.length === 0 ||
    normalized === "tool" ||
    normalized === "tool_call" ||
    normalized === "mcp_tool_call" ||
    normalized === "unknown" ||
    normalized === "read" ||
    normalized === "write" ||
    normalized === "execute" ||
    normalized === "completed"
  );
}

export function formatToolTitle(
  toolName: string,
  fallbackTitle?: string,
): string {
  const name = normalizeToolName(toolName);
  if (SPROUT_READ_TOOLS.has(name) || SPROUT_WRITE_TOOLS.has(name)) {
    return name
      .split("_")
      .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
      .join(" ");
  }
  if (fallbackTitle && !isGenericToolTitle(fallbackTitle)) {
    return fallbackTitle;
  }
  return toolName;
}
