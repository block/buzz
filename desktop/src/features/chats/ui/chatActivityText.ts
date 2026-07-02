import type { TranscriptItem } from "@/features/agents/ui/agentSessionTypes";
import { buildCompactToolSummary } from "@/features/agents/ui/agentSessionToolSummary";

export type ActivityMarkerTone =
  | "default"
  | "muted"
  | "success"
  | "warning"
  | "danger";

export function cleanAssistantMessageText(text: string) {
  return stripChatModeEmoji(text);
}

export function normalizeAssistantMessageTextForMatching(text: string) {
  return cleanAssistantMessageText(text).replace(/\s+/g, " ").trim();
}

export function isHumanFacingAssistantText(text: string) {
  const normalized = normalizeAssistantMessageTextForMatching(text);
  return normalized.length > 0 && !isAgentInternalNarration(normalized);
}

export function cleanChatMessageText(
  item: Extract<TranscriptItem, { type: "message" }>,
) {
  const text = item.text.trim();
  if (item.role !== "assistant") {
    return text;
  }
  return cleanAssistantMessageText(text);
}

export function isHumanFacingAssistantMessage(
  item: Extract<TranscriptItem, { type: "message" }>,
) {
  const text = normalizeAssistantMessageTextForMatching(
    cleanChatMessageText(item),
  );
  return text.length > 0 && !isAgentInternalNarration(text);
}

export function toolLabel(item: Extract<TranscriptItem, { type: "tool" }>) {
  const summary = buildCompactToolSummary(item);
  const preview = summary.preview?.trim();
  const isRunning = item.status === "executing" || item.status === "pending";
  if (summary.kind === "shell") {
    if (isRunning) {
      return ["Running", preview].filter(Boolean).join(" ");
    }
    if (item.isError || item.status === "failed") {
      return "Command failed";
    }
    return "Ran command";
  }
  if (summary.action?.verb === "Searched") {
    return isRunning
      ? ["Searching", preview].filter(Boolean).join(" ")
      : "Searched";
  }
  if (summary.kind === "message") {
    return "Sent message";
  }
  if (summary.action) {
    return [summary.action.verb, summary.action.object ?? summary.preview]
      .filter(Boolean)
      .join(" ");
  }
  if (summary.fileEditSummary) {
    return `Edited ${summary.fileEditSummary.filename}`;
  }
  return [summary.label, summary.preview].filter(Boolean).join(" · ");
}

export function completedWorkLabel(items: TranscriptItem[]) {
  const duration = workDuration(items);
  return duration ? `Thought for ${duration}` : "Thought";
}

export function activityItemLabel(item: TranscriptItem) {
  if (item.type === "tool") {
    return toolLabel(item);
  }
  if (item.type === "metadata") {
    return `Captured ${item.title.toLowerCase()}`;
  }
  if (item.type === "plan") {
    return item.isUpdate
      ? item.text
        ? `Updated plan · ${item.text}`
        : "Updated plan"
      : "Updated plan";
  }
  if (item.type === "thought") {
    return item.title || "Thinking";
  }
  if (item.type === "lifecycle") {
    return item.title;
  }
  if (item.type === "message" && item.role === "user") {
    return "User prompt";
  }
  return "";
}

export function activityItemTone(item: TranscriptItem): ActivityMarkerTone {
  if (item.type === "tool") {
    if (item.isError || item.status === "failed") return "danger";
    if (item.status === "executing" || item.status === "pending") {
      return "warning";
    }
    return "default";
  }
  if (item.type === "lifecycle" && item.renderClass === "error") {
    return "danger";
  }
  return "muted";
}

function stripChatModeEmoji(text: string) {
  return text
    .trim()
    .replace(/[\p{Emoji_Presentation}\p{Extended_Pictographic}]/gu, "")
    .replace(/[ \t]{2,}/g, " ")
    .replace(/[ \t]+\n/g, "\n")
    .trim();
}

function isAgentInternalNarration(text: string) {
  return AGENT_INTERNAL_NARRATION_PATTERNS.some((pattern) =>
    pattern.test(text),
  );
}

const AGENT_INTERNAL_NARRATION_PATTERNS = [
  /^now\s+let\s+me\b/i,
  /^let\s+me\s+(?:also\s+)?(?:check|create|find|grab|inspect|look|open|pull|read|review|run|search|see|verify)\b/i,
  /^(?:replied|replying|responded|sent|posted)\s+to\s+.+(?:\b(?:message|thread|channel)\b|[, ]\s*(?:confirming|saying|telling|noting|letting|ready|with|that)\b)/i,
  /^(?:asked|told|replied|responded)\s+(?:kenny|kenneth|the\s+user|him|her|them)\b/i,
  /^(?:i\s+)?(?:asked|told|replied\s+to|responded\s+to)\s+(?:kenny|kenneth|the\s+user|him|her|them)\b/i,
  /^(?:now\s+)?(?:i|we)\s+(?:can\s+see|have|found|got|gathered|collected|see)\b.+\blet\s+me\s+(?:(?:also\s+)?(?:check|create|find|inspect|look|verify|review|search|run)|(?:reply|send|post)\b)/i,
  /^(?:done[!.]?\s+)?(?:i|we)(?:'ve|\s+have|\s+just)?\s+(?:sent|posted)\b.+\b(?:breakdown|summary|details?|reply|response|answer)\b/i,
  /^(?:i|we)(?:'ll|\s+will|\s+am\s+going\s+to|\s+are\s+going\s+to)\s+(?:send|post)\b/i,
  /^let\s+me\s+(?:reply|send|post)\b/i,
  /^that(?:'s|\s+is)\b.+\blet\s+me\b/i,
  /^(?:the|this|that)\s+(?:message|request|prompt)\b.+\b(?:top-level|thread|channel|reply|ambiguous|ambitious|could\s+mean|likely\s+means|seems\s+to\s+mean|clarification)\b/i,
  /^(?:kenny|kenneth|the\s+user|he|she|they)\s+(?:asked|wants|means|could\s+mean|might\s+mean|is\s+asking|is\s+referring)\b/i,
  /^(?:i|we)\s+(?:need\s+to|should|can|will)\s+(?:ask|request)\s+(?:kenny|kenneth|the\s+user|him|her|them)\s+for\s+clarification\b/i,
] satisfies RegExp[];

function workDuration(items: TranscriptItem[]) {
  const times = items
    .map((item) => Date.parse(item.timestamp))
    .filter((value) => Number.isFinite(value));
  if (times.length < 2) {
    return null;
  }
  const seconds = Math.max(
    1,
    Math.round((Math.max(...times) - Math.min(...times)) / 1000),
  );
  if (seconds < 60) {
    return `${seconds}s`;
  }
  const minutes = Math.floor(seconds / 60);
  const remainder = seconds % 60;
  return remainder > 0 ? `${minutes}m ${remainder}s` : `${minutes}m`;
}
