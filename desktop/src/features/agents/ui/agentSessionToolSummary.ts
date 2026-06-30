import type { ToolStatus, TranscriptItem } from "./agentSessionTypes";
import type { AgentActivityDescriptor } from "./agentSessionTypes";
import {
  asRecord,
  getToolString,
  parseToolResultValue,
} from "./agentSessionUtils";
import { classifyToolItem } from "./agentSessionToolClassifier";

export type CompactToolKind =
  | "message"
  | "relay-op"
  | "file-edit"
  | "shell"
  | "status"
  | "thought"
  | "plan"
  | "permission"
  | "error"
  | "generic"
  | "raw-rail"
  | "suppressed";

export type CompactToolSummary = {
  kind: CompactToolKind;
  label: string;
  preview: string | null;
  fileEditSummary: CompactFileEditSummary | null;
  /** When set, the compact row renders a tiny image instead of text preview. */
  thumbnailSrc: string | null;
  presentation: "inline" | "message";
  descriptor: AgentActivityDescriptor;
};

type ToolItem = Extract<TranscriptItem, { type: "tool" }>;

export type CompactFileEditSummary = {
  path: string;
  filename: string;
  additions: number;
  deletions: number;
};

/** Build the muted compact summary label and preview for any tool row. */
export function buildCompactToolSummary(item: ToolItem): CompactToolSummary {
  const descriptor = item.descriptor ?? classifyToolItem(item);
  const fileEditSummary = getFileEditSummary(item, descriptor);
  const thumbnailSrc = getThumbnailSrc(item, descriptor);
  const failed = item.isError || item.status === "failed";
  const running = item.status === "executing" || item.status === "pending";
  return {
    kind: descriptor.renderClass,
    label: labelForStatus(descriptor, item.status, failed, running),
    preview: fileEditSummary?.filename ?? descriptor.preview,
    fileEditSummary,
    thumbnailSrc,
    presentation: descriptor.renderClass === "message" ? "message" : "inline",
    descriptor,
  };
}

function labelForStatus(
  descriptor: AgentActivityDescriptor,
  status: ToolStatus,
  failed: boolean,
  running: boolean,
) {
  const label = descriptor.label;
  if (descriptor.groupKey === "file-edit:str_replace") {
    if (failed) return "Edit failed";
    if (running) return "Editing file";
    return "Edited file";
  }
  if (failed) {
    return label.endsWith("failed") ? label : `${label} failed`;
  }
  if (running) return label;
  if (status === "completed") return label;
  return label;
}

function getThumbnailSrc(
  item: ToolItem,
  descriptor: AgentActivityDescriptor,
): string | null {
  const operation =
    descriptor.operation ?? descriptor.groupKey ?? item.toolName;
  if (!operation.includes("view_image") && item.toolName !== "view_image") {
    return null;
  }

  const source = getToolString(item.args, ["source"]);
  if (!source) return null;
  const trimmed = source.trim();
  return trimmed.startsWith("data:image/") ||
    trimmed.startsWith("http://") ||
    trimmed.startsWith("https://")
    ? trimmed
    : null;
}

function getFileEditSummary(
  item: ToolItem,
  descriptor: AgentActivityDescriptor,
): CompactFileEditSummary | null {
  if (descriptor.renderClass !== "file-edit") {
    return null;
  }

  const resultText = getResultText(item.result);
  const path =
    getToolString(item.args, ["path", "file", "file_path", "target_file"]) ??
    descriptor.object ??
    descriptor.preview ??
    getDiffPath(resultText);

  if (!path) {
    return null;
  }

  const stats = getDiffStats(resultText);
  if (!stats) {
    return null;
  }

  return {
    path,
    filename: basename(path),
    additions: stats.additions,
    deletions: stats.deletions,
  };
}

function getResultText(result: string): string {
  const parsed = parseToolResultValue(result);
  if (typeof parsed === "string") {
    return parsed;
  }

  const record = asRecord(parsed);
  return [
    getToolString(record, ["stdout", "output", "text"]),
    getToolString(record, ["stderr"]),
    result,
  ]
    .filter((value): value is string => value != null)
    .join("\n");
}

function getDiffPath(text: string): string | null {
  for (const line of text.split(/\r?\n/)) {
    const match = line.match(/^\+\+\+\s+(?:b\/)?(.+)$/);
    if (match?.[1] && match[1] !== "/dev/null") {
      return match[1].trim();
    }
  }

  for (const line of text.split(/\r?\n/)) {
    const match = line.match(/^---\s+(?:a\/)?(.+)$/);
    if (match?.[1] && match[1] !== "/dev/null") {
      return match[1].trim();
    }
  }

  return null;
}

function getDiffStats(
  text: string,
): Pick<CompactFileEditSummary, "additions" | "deletions"> | null {
  let additions = 0;
  let deletions = 0;

  for (const line of text.split(/\r?\n/)) {
    if (/\s*\/\/\s*\[!code\s*\+\+\]\s*$/.test(line)) {
      additions += 1;
      continue;
    }
    if (/\s*\/\/\s*\[!code\s*--\]\s*$/.test(line)) {
      deletions += 1;
      continue;
    }
    if (line.startsWith("+++") || line.startsWith("---")) {
      continue;
    }
    if (line.startsWith("+")) {
      additions += 1;
      continue;
    }
    if (line.startsWith("-")) {
      deletions += 1;
    }
  }

  if (additions > 0 || deletions > 0) {
    return { additions, deletions };
  }

  const statAdditions = text.match(/(\d+)\s+insertions?\(\+\)/);
  const statDeletions = text.match(/(\d+)\s+deletions?\(-\)/);
  if (statAdditions || statDeletions) {
    return {
      additions: statAdditions ? Number(statAdditions[1]) : 0,
      deletions: statDeletions ? Number(statDeletions[1]) : 0,
    };
  }

  return null;
}

function basename(path: string) {
  const parts = path.replace(/\\/g, "/").split("/");
  return parts[parts.length - 1] || path;
}
