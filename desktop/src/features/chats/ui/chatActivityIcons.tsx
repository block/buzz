import {
  AlertTriangle,
  CheckCircle2,
  MessageCircle,
  Pencil,
  Search,
  SquareTerminal,
} from "lucide-react";

import type { TranscriptSameKindSummary } from "@/features/agents/ui/agentSessionTranscriptGrouping";
import type { TranscriptItem } from "@/features/agents/ui/agentSessionTypes";
import { buildCompactToolSummary } from "@/features/agents/ui/agentSessionToolSummary";
import { getToolString } from "@/features/agents/ui/agentSessionUtils";

type ToolItem = Extract<TranscriptItem, { type: "tool" }>;
type CompactToolSummary = ReturnType<typeof buildCompactToolSummary>;

export function activityItemIcon(item: TranscriptItem) {
  if (item.type === "tool") {
    return toolIcon(item, buildCompactToolSummary(item));
  }
  return null;
}

export function toolIcon(item: ToolItem, summary: CompactToolSummary) {
  if (item.isError || item.status === "failed") {
    return <AlertTriangle className="h-3.5 w-3.5" />;
  }
  return toolCategoryIcon(item, summary);
}

export function toolCategoryIcon(item: ToolItem, summary: CompactToolSummary) {
  const kind = summary.kind;
  if (kind === "file-edit") {
    return <Pencil className="h-3.5 w-3.5" />;
  }
  if (isSearchTool(item)) {
    return <Search className="h-3.5 w-3.5" />;
  }
  if (isShellCommandTool(item, summary)) {
    return <SquareTerminal className="h-3.5 w-3.5" />;
  }
  if (kind === "message") {
    return <MessageCircle className="h-3.5 w-3.5" />;
  }
  return <CheckCircle2 className="h-3.5 w-3.5" />;
}

export function summaryIcon(summary: TranscriptSameKindSummary) {
  if (summary.renderClass === "file-edit") {
    return <Pencil className="h-3.5 w-3.5" />;
  }
  if (
    summary.items.some((item) => item.type === "tool" && isSearchTool(item))
  ) {
    return <Search className="h-3.5 w-3.5" />;
  }
  if (
    summary.renderClass === "shell" ||
    summary.items.some(
      (item) =>
        item.type === "tool" &&
        isShellCommandTool(item, buildCompactToolSummary(item)),
    )
  ) {
    return <SquareTerminal className="h-3.5 w-3.5" />;
  }
  return <CheckCircle2 className="h-3.5 w-3.5" />;
}

export function isSearchTool(item: ToolItem) {
  return buildCompactToolSummary(item).action?.verb === "Searched";
}

export function isShellOriginTool(summary: CompactToolSummary) {
  return (
    summary.kind === "shell" ||
    summary.descriptor.source === "shell" ||
    summary.descriptor.groupKey?.startsWith("shell:") === true
  );
}

export function isShellCommandTool(
  item: ToolItem,
  summary: CompactToolSummary,
) {
  return (
    isShellOriginTool(summary) || getToolString(item.args, ["command"]) !== null
  );
}

export function getShellCommand(item: ToolItem, summary: CompactToolSummary) {
  if (!isShellCommandTool(item, summary)) {
    return null;
  }
  return getToolString(item.args, ["command"]) ?? summary.preview ?? "command";
}
