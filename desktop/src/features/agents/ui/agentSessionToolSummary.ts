import type {
  AgentActivityAction,
  ToolStatus,
  TranscriptItem,
} from "./agentSessionTypes";
import type { AgentActivityDescriptor } from "./agentSessionTypes";
import { getToolString } from "./agentSessionUtils";
import { classifyToolItem } from "./agentSessionToolClassifier";
import {
  buildFileEditDiff,
  type FileEditDiff,
  type FileEditDiffSummary,
} from "./agentSessionFileEditDiff";
import {
  buildFileReadContent,
  type FileReadContent,
} from "./agentSessionFileRead";

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
  action: AgentActivityAction | null;
  kind: CompactToolKind;
  label: string;
  preview: string | null;
  fileEditSummary: FileEditDiffSummary | null;
  fileEditDiff: FileEditDiff | null;
  fileReadContent: FileReadContent | null;
  /** When set, the compact row renders a tiny image instead of text preview. */
  thumbnailSrc: string | null;
  presentation: "inline" | "message";
  descriptor: AgentActivityDescriptor;
};

type ToolItem = Extract<TranscriptItem, { type: "tool" }>;

export type CompactFileEditSummary = FileEditDiffSummary;

/** Build the muted compact summary label and preview for any tool row. */
export function buildCompactToolSummary(item: ToolItem): CompactToolSummary {
  const descriptor = item.descriptor ?? classifyToolItem(item);
  const fileEditDiff = buildFileEditDiff(item, descriptor);
  const fileEditSummary = fileEditDiff
    ? {
        path: fileEditDiff.path,
        filename: fileEditDiff.filename,
        additions: fileEditDiff.additions,
        deletions: fileEditDiff.deletions,
      }
    : null;
  const fileReadContent = buildFileReadContent(item, descriptor);
  const thumbnailSrc = getThumbnailSrc(item, descriptor);
  const failed = item.isError || item.status === "failed";
  const running = item.status === "executing" || item.status === "pending";
  return {
    action: descriptor.action ?? null,
    kind: descriptor.renderClass,
    label: labelForStatus(descriptor, item.status, failed, running),
    preview: fileEditSummary?.filename ?? descriptor.preview,
    fileEditSummary,
    fileEditDiff,
    fileReadContent,
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
