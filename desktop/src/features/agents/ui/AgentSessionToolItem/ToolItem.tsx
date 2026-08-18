import * as React from "react";

import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import type { TranscriptItem } from "../agentSessionTypes";
import { getBuzzToolInfo } from "../agentSessionToolCatalog";
import { buildCompactToolSummary } from "../agentSessionToolSummary";
import type { AgentTranscriptIdentityProps } from "../activityRenderClasses/types";
import {
  formatTranscriptTimestampTitle,
  getToolDurationDisplay,
} from "../agentSessionUtils";
import { CompactMessageSummary } from "./CompactMessageSummary";
import {
  CompactToolSummaryRow,
  compactSummaryTone,
} from "./CompactToolSummaryRow";
import { getSentMessageLink } from "./messageLinks";
import { isTodoSummary, TodoToolSummary } from "./TodoToolSummary";
import { ToolDetailBlocks } from "./ToolDetailBlocks";

export function ToolItem({
  agentAvatarUrl,
  agentName,
  agentPubkey,
  item,
  profiles,
}: AgentTranscriptIdentityProps & {
  item: Extract<TranscriptItem, { type: "tool" }>;
  profiles?: UserProfileLookup;
}) {
  const [isExpanded, setIsExpanded] = React.useState(false);
  const hasArgs = Object.keys(item.args).length > 0;
  const hasResult = item.result.trim().length > 0;
  const canonicalToolName = item.buzzToolName ?? item.toolName;
  const buzzTool = getBuzzToolInfo(canonicalToolName);
  const compactSummary = buildCompactToolSummary(item);
  const duration = getToolDurationDisplay(item);
  const messageLink = getSentMessageLink(item);
  const timestampTitle = formatTranscriptTimestampTitle(item.timestamp);
  const agentProfile = profiles?.[normalizePubkey(agentPubkey)] ?? null;
  const agentLabel = resolveUserLabel({
    pubkey: agentPubkey,
    fallbackName: agentName,
    profiles,
    preferResolvedSelfLabel: true,
  });
  const agentResolvedAvatarUrl = agentProfile?.avatarUrl ?? agentAvatarUrl;
  const handleToggle = React.useCallback(
    (event: React.SyntheticEvent<HTMLDetailsElement>) => {
      setIsExpanded(event.currentTarget.open);
    },
    [],
  );

  if (compactSummary.presentation === "message") {
    return (
      <div
        className="not-prose w-full"
        data-testid="transcript-tool-item"
        title={timestampTitle}
      >
        <CompactMessageSummary
          args={item.args}
          avatarUrl={agentResolvedAvatarUrl}
          description={buzzTool?.label}
          displayName={agentLabel}
          duration={duration}
          hasArgs={hasArgs}
          hasResult={hasResult}
          isError={item.isError || item.status === "failed"}
          label={compactSummary.label}
          messageLink={messageLink}
          preview={compactSummary.preview}
          pubkey={agentPubkey}
          result={item.result}
          timestamp={item.timestamp}
        />
      </div>
    );
  }

  if (isTodoSummary(compactSummary)) {
    return (
      <div
        className="not-prose w-full"
        data-testid="transcript-tool-item"
        title={timestampTitle}
      >
        <TodoToolSummary
          duration={duration}
          fallbackPreview={compactSummary.preview}
          item={item}
        />
      </div>
    );
  }

  return (
    <div
      className="not-prose w-full"
      data-testid="transcript-tool-item"
      title={timestampTitle}
    >
      <details
        className="group w-full"
        onToggle={handleToggle}
        open={isExpanded}
      >
        <summary
          className={cn(
            "group/row flex min-h-6 max-w-full cursor-pointer list-none items-center gap-1.5",
            compactSummaryTone(),
          )}
        >
          <CompactToolSummaryRow
            action={compactSummary.action}
            duration={duration}
            fileEditSummary={compactSummary.fileEditSummary}
            kind={compactSummary.kind}
            preview={compactSummary.preview}
            thumbnailSrc={compactSummary.thumbnailSrc}
            label={compactSummary.label}
          />
        </summary>

        <ToolDetailBlocks
          args={item.args}
          description={buzzTool?.label}
          fileEditDiff={compactSummary.fileEditDiff}
          fileReadContent={compactSummary.fileReadContent}
          hasArgs={hasArgs}
          hasResult={hasResult}
          imagePreview={
            compactSummary.imageContent != null && isExpanded
              ? {
                  src: compactSummary.imageContent.src,
                  title: compactSummary.imageContent.title,
                }
              : null
          }
          isError={item.isError}
          result={item.result}
          shellCommand={compactSummary.shellContent}
        />
      </details>
    </div>
  );
}
