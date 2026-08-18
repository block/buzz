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
  buildSkillReadContent,
  type FileReadContent,
} from "./agentSessionFileRead";
import {
  buildImageContent,
  type ImageToolContent,
} from "./agentSessionImageContent";

export type CompactToolKind =
  | "message"
  | "relay-op"
  | "file-edit"
  | "file-read"
  | "skill-read"
  | "image"
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
  imageContent: ImageToolContent | null;
  shellContent: string | null;
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
  const fileReadContent =
    buildFileReadContent(item, descriptor) ??
    buildSkillReadContent(item, descriptor);
  const imageContent = buildImageContent(item, descriptor);
  const shellContent = buildShellContent(item, descriptor);
  const thumbnailSrc = imageContent?.src ?? null;
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
    imageContent,
    shellContent,
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

function buildShellContent(
  item: ToolItem,
  descriptor: AgentActivityDescriptor,
): string | null {
  const command = getToolString(item.args, ["command"]);
  if (!command) {
    return null;
  }

  if (descriptor.renderClass === "shell" || descriptor.source === "shell") {
    return command;
  }

  return null;
}
