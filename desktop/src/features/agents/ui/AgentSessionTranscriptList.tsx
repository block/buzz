import * as React from "react";
import { CheckCheck, Radio } from "lucide-react";

import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { useAnchoredScroll } from "@/features/messages/ui/useAnchoredScroll";
import { cn } from "@/shared/lib/cn";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Spinner } from "@/shared/ui/spinner";
import type { TranscriptItem } from "./agentSessionTypes";
import { PromptSectionList as PromptContextSections } from "./PromptSectionAccordion";
import {
  AgentSessionTranscriptVariantProvider,
  type AgentSessionTranscriptVariant,
  useAgentSessionTranscriptVariant,
} from "./agentSessionTranscriptContext";
import { TranscriptActivityItem } from "./activityRenderClasses/TranscriptActivityItem";
import {
  ActivityRow,
  ActivityRowContent,
  ActivityRowLabel,
  type ActivityRowStats,
  splitActivityRowLabel,
} from "./activityRenderClasses/ActivityRow";
import { TranscriptTimestamp } from "./activityRenderClasses/TranscriptTimestamp";
import type { AgentTranscriptIdentityProps } from "./activityRenderClasses/types";
import type { FileEditDiff } from "./agentSessionFileEditDiff";
import {
  buildTranscriptDisplayBlocks,
  formatTurnSetupLabel,
  turnSetupDetail,
  turnSetupTimestamp,
  type TranscriptDisplayBlock,
  type TranscriptTurnSegment,
} from "./agentSessionTranscriptGrouping";
import { buildCompactToolSummary } from "./agentSessionToolSummary";
import { formatTranscriptTimestampTitle } from "./agentSessionUtils";
import { hasFileEditLineDiff } from "./FileEditDiffView";
import { UserMessageBubble } from "./activityRenderClasses/UserMessageBubble";

const TRANSCRIPT_ACP_SOURCE_STORAGE_KEY = "buzz:show-transcript-acp-source";

/**
 * Opt-in only: source pills are useful while iterating on observer parsing, but
 * they should not appear for every local dev session.
 */
const SHOW_TRANSCRIPT_ACP_SOURCE = shouldShowTranscriptAcpSource();

export type AgentSessionTranscriptEmptyState = "idle" | "loading";

function shouldShowTranscriptAcpSource() {
  const envValue = import.meta.env.VITE_SHOW_TRANSCRIPT_ACP_SOURCE;
  if (envValue === "1" || envValue === "true") {
    return true;
  }

  if (typeof window === "undefined") {
    return false;
  }

  try {
    return (
      window.localStorage.getItem(TRANSCRIPT_ACP_SOURCE_STORAGE_KEY) === "1"
    );
  } catch {
    return false;
  }
}

export function AgentSessionTranscriptList({
  agentAvatarUrl,
  agentName,
  agentPubkey,
  autoTail = false,
  emptyDescription,
  emptyState = "idle",
  items,
  profiles,
  contentContainerClassName,
  scrollScopeKey,
  variant = "default",
}: AgentTranscriptIdentityProps & {
  autoTail?: boolean;
  emptyDescription: string;
  emptyState?: AgentSessionTranscriptEmptyState;
  items: TranscriptItem[];
  profiles?: UserProfileLookup;
  contentContainerClassName?: string;
  scrollScopeKey?: string | null;
  variant?: AgentSessionTranscriptVariant;
}) {
  const displayBlocks = React.useMemo(
    () => buildTranscriptDisplayBlocks(items),
    [items],
  );
  const scrollContainerRef = React.useRef<HTMLDivElement>(null);
  const contentRef = React.useRef<HTMLDivElement>(null);
  const anchoredScroll = useAnchoredScroll({
    channelId: autoTail ? (scrollScopeKey ?? agentPubkey) : null,
    contentRef,
    isLoading: false,
    messages: items,
    scrollContainerRef,
  });

  React.useLayoutEffect(() => {
    if (!autoTail || items.length === 0) {
      return;
    }

    anchoredScroll.scrollToBottom("auto");
  }, [anchoredScroll.scrollToBottom, autoTail, items]);

  const scrollContainerClassNames = cn(
    "w-full",
    autoTail ? "h-full overflow-y-auto" : null,
  );

  if (items.length === 0) {
    const isLoading = emptyState === "loading";

    return (
      <div className={scrollContainerClassNames}>
        <div className="flex min-h-40 flex-col items-center justify-center px-6 py-10 text-center">
          {isLoading ? (
            <Spinner
              aria-label="Waiting for ACP activity"
              className="mx-auto h-4 w-4 border-2 text-muted-foreground"
            />
          ) : (
            <Radio className="mx-auto h-4 w-4 text-muted-foreground" />
          )}
          <p className="mt-3 text-sm font-medium">
            {isLoading ? "Waiting for ACP activity" : "No ACP activity yet"}
          </p>
          <p className="mt-1 text-sm text-muted-foreground">
            {emptyDescription}
          </p>
        </div>
      </div>
    );
  }

  const isCompactPreview = variant === "compactPreview";

  return (
    <div
      className={scrollContainerClassNames}
      onScroll={autoTail ? anchoredScroll.onScroll : undefined}
      ref={autoTail ? scrollContainerRef : undefined}
    >
      <div
        aria-label="Live ACP transcript"
        aria-live="polite"
        className={cn(
          "flex w-full flex-col",
          isCompactPreview ? "gap-2" : "gap-4",
          autoTail && "pb-4",
          contentContainerClassName,
        )}
        ref={autoTail ? contentRef : undefined}
        role="log"
      >
        <AgentSessionTranscriptVariantProvider value={variant}>
          {displayBlocks.map((block) => (
            <div
              className="content-visibility-auto"
              data-message-id={getDisplayBlockKey(block)}
              key={getDisplayBlockKey(block)}
            >
              <TranscriptDisplayBlockView
                agentAvatarUrl={agentAvatarUrl}
                agentName={agentName}
                agentPubkey={agentPubkey}
                block={block}
                profiles={profiles}
              />
            </div>
          ))}
        </AgentSessionTranscriptVariantProvider>
      </div>
    </div>
  );
}

function TranscriptAcpSourceBadge({ source }: { source: string }) {
  return (
    <span
      className="mb-1 inline-flex max-w-full rounded border border-amber-500/25 bg-amber-500/10 px-1.5 py-0.5 font-mono text-xs leading-none text-amber-800 dark:text-amber-200"
      data-testid="transcript-acp-source"
      title={`ACP wire source: ${source}`}
    >
      {source}
    </span>
  );
}

function getDisplayBlockKey(block: TranscriptDisplayBlock) {
  if (block.kind === "single") {
    return block.item.id;
  }
  return `turn:${block.turnId}`;
}

function TranscriptDisplayBlockView({
  agentAvatarUrl,
  agentName,
  agentPubkey,
  block,
  profiles,
}: AgentTranscriptIdentityProps & {
  block: TranscriptDisplayBlock;
  profiles?: UserProfileLookup;
}) {
  const variant = useAgentSessionTranscriptVariant();
  const isCompactPreview = variant === "compactPreview";

  if (block.kind === "single") {
    return (
      <TranscriptItemRow
        agentAvatarUrl={agentAvatarUrl}
        agentName={agentName}
        agentPubkey={agentPubkey}
        item={block.item}
        profiles={profiles}
      />
    );
  }

  return (
    <div
      className={cn("flex flex-col", isCompactPreview ? "gap-2.5" : "gap-4")}
      data-testid="transcript-turn-group"
      data-turn-id={block.turnId}
    >
      {block.segments.map((segment) => (
        <TranscriptTurnSegmentView
          agentAvatarUrl={agentAvatarUrl}
          agentName={agentName}
          agentPubkey={agentPubkey}
          key={getTurnSegmentKey(block.turnId, segment)}
          profiles={profiles}
          segment={segment}
        />
      ))}
    </div>
  );
}

function getTurnSegmentKey(turnId: string, segment: TranscriptTurnSegment) {
  if (segment.kind === "setup") {
    return `turn:${turnId}:setup`;
  }
  if (segment.kind === "prompt") {
    return `turn:${turnId}:prompt`;
  }
  if (segment.kind === "summary") {
    return segment.summary.id;
  }
  return segment.item.id;
}

function TranscriptTurnSegmentView({
  agentAvatarUrl,
  agentName,
  agentPubkey,
  profiles,
  segment,
}: AgentTranscriptIdentityProps & {
  profiles?: UserProfileLookup;
  segment: TranscriptTurnSegment;
}) {
  if (segment.kind === "prompt") {
    return (
      <TurnPromptBlock
        context={segment.context}
        profiles={profiles}
        setup={segment.setup}
        systemPrompt={segment.systemPrompt}
        user={segment.user}
      />
    );
  }

  if (segment.kind === "setup") {
    return <TurnSetupStatus items={segment.items} />;
  }

  if (segment.kind === "summary") {
    return (
      <SameKindSummaryItem
        agentAvatarUrl={agentAvatarUrl}
        agentName={agentName}
        agentPubkey={agentPubkey}
        profiles={profiles}
        summary={segment.summary}
      />
    );
  }

  return (
    <TranscriptItemRow
      agentAvatarUrl={agentAvatarUrl}
      agentName={agentName}
      agentPubkey={agentPubkey}
      item={segment.item}
      profiles={profiles}
    />
  );
}

function SameKindSummaryItem({
  agentAvatarUrl,
  agentName,
  agentPubkey,
  profiles,
  summary,
}: AgentTranscriptIdentityProps & {
  profiles?: UserProfileLookup;
  summary: Extract<TranscriptTurnSegment, { kind: "summary" }>["summary"];
}) {
  const groupedFileEditDiffs = React.useMemo(
    () =>
      summary.renderClass === "file-edit"
        ? getGroupedFileEditDiffs(summary.items)
        : [],
    [summary.items, summary.renderClass],
  );
  const groupedFileEditStats = summarizeFileEditDiffs(groupedFileEditDiffs);
  const expandsToToolItems = summary.items.every(
    (item) => item.type === "tool",
  );

  return (
    <ActivityRow
      className="flex flex-col gap-0.5"
      openToneScope="summary"
      testId="transcript-same-kind-summary"
      title={formatTranscriptTimestampTitle(summary.timestamp)}
    >
      <ToolRunSummaryLabel label={summary.label} stats={groupedFileEditStats} />
      <ActivityRowContent
        className={cn(
          "flex flex-col",
          expandsToToolItems ? "gap-0.5" : "gap-1 pl-5",
        )}
      >
        {expandsToToolItems
          ? summary.items.map((item) => (
              <TranscriptItemView
                agentAvatarUrl={agentAvatarUrl}
                agentName={agentName}
                agentPubkey={agentPubkey}
                item={item}
                key={item.id}
                profiles={profiles}
              />
            ))
          : summary.items.map((item) => (
              <p
                className="truncate text-xs text-muted-foreground"
                key={item.id}
              >
                {item.type === "tool"
                  ? item.descriptor.preview || item.descriptor.label
                  : item.title}
              </p>
            ))}
      </ActivityRowContent>
    </ActivityRow>
  );
}

function getGroupedFileEditDiffs(items: TranscriptItem[]): FileEditDiff[] {
  return items.flatMap((item) => {
    if (item.type !== "tool" || item.isError) {
      return [];
    }

    const diff = buildCompactToolSummary(item).fileEditDiff;
    return diff && hasFileEditLineDiff(diff) ? [diff] : [];
  });
}

function summarizeFileEditDiffs(
  diffs: FileEditDiff[],
): ActivityRowStats | null {
  if (diffs.length === 0) {
    return null;
  }

  return diffs.reduce(
    (stats, diff) => ({
      additions: stats.additions + diff.additions,
      deletions: stats.deletions + diff.deletions,
    }),
    { additions: 0, deletions: 0 },
  );
}

function ToolRunSummaryLabel({
  label,
  stats,
}: {
  label: string;
  stats?: ActivityRowStats | null;
}) {
  const parts = splitActivityRowLabel(label);

  if (!parts) {
    return <span className="truncate text-sm font-medium">{label}</span>;
  }

  return (
    <ActivityRowLabel
      object={parts.object}
      openToneScope="summary"
      stats={stats}
      verb={parts.verb}
    />
  );
}

function TurnPromptBlock({
  context,
  profiles,
  setup,
  systemPrompt,
  user,
}: {
  context: Extract<TranscriptItem, { type: "metadata" }> | null;
  profiles?: UserProfileLookup;
  setup: Extract<TranscriptItem, { type: "lifecycle" }>[];
  systemPrompt: Extract<TranscriptItem, { type: "metadata" }> | null;
  user: Extract<TranscriptItem, { type: "message" }>;
}) {
  return (
    <div data-testid="transcript-prompt-bundle">
      {SHOW_TRANSCRIPT_ACP_SOURCE ? (
        <div className="mb-1 flex flex-wrap gap-1">
          <TranscriptAcpSourceBadge source="session/prompt:user" />
          {context ? (
            <TranscriptAcpSourceBadge source="session/prompt:context" />
          ) : null}
        </div>
      ) : null}
      <PromptUserMessage
        context={context}
        item={user}
        profiles={profiles}
        setup={setup}
        systemPrompt={systemPrompt}
      />
    </div>
  );
}

function PromptUserMessage({
  context = null,
  item,
  profiles,
  setup = [],
  systemPrompt = null,
}: {
  context?: Extract<TranscriptItem, { type: "metadata" }> | null;
  item: Extract<TranscriptItem, { type: "message" }>;
  profiles?: UserProfileLookup;
  setup?: Extract<TranscriptItem, { type: "lifecycle" }>[];
  systemPrompt?: Extract<TranscriptItem, { type: "metadata" }> | null;
}) {
  return (
    <>
      <UserMessageBubble
        bubbleClassName="p-2.5"
        footer={
          <TurnSetupFooter
            items={setup}
            messageLink={getTranscriptMessageLink(item)}
            timestamp={item.timestamp}
          />
        }
        item={item}
        profiles={profiles}
      />
      {systemPrompt && systemPrompt.sections.length > 0 ? (
        <PromptContextInline context={systemPrompt} />
      ) : null}
      {context && context.sections.length > 0 ? (
        <PromptContextInline context={context} />
      ) : null}
    </>
  );
}

function PromptContextInline({
  context,
}: {
  context: Extract<TranscriptItem, { type: "metadata" }>;
}) {
  const [dialogOpen, setDialogOpen] = React.useState(false);

  return (
    <>
      <div
        className="mt-1 space-y-2 pl-2"
        data-testid="transcript-prompt-context-inline"
      >
        <div className="flex items-center justify-between gap-2">
          <p className="text-xs font-medium text-muted-foreground/70">
            {context.title}
          </p>
          <button
            className="text-xs text-muted-foreground/50 hover:text-muted-foreground transition-colors"
            data-testid="transcript-prompt-context-expand"
            onClick={() => setDialogOpen(true)}
            type="button"
          >
            View full
          </button>
        </div>
        <PromptContextSections sections={context.sections} />
      </div>
      <PromptContextDialog
        context={context}
        onOpenChange={setDialogOpen}
        open={dialogOpen}
      />
    </>
  );
}

function PromptContextDialog({
  context,
  onOpenChange,
  open,
}: {
  context: Extract<TranscriptItem, { type: "metadata" }>;
  onOpenChange: (open: boolean) => void;
  open: boolean;
}) {
  if (!open || context.sections.length === 0) {
    return null;
  }

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="max-w-xl overflow-hidden p-0">
        <div className="flex max-h-[85vh] flex-col">
          <DialogHeader className="px-6 pb-3 pt-5 pr-14">
            <DialogTitle>{context.title}</DialogTitle>
          </DialogHeader>
          <div className="min-h-0 flex-1 overflow-y-auto px-6 pb-6 pt-2">
            <PromptContextSections sections={context.sections} />
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function TurnSetupFooter({
  items,
  messageLink = null,
  showTimestamp = true,
  timestamp,
}: {
  items: Extract<TranscriptItem, { type: "lifecycle" }>[];
  messageLink?: { channelId: string; messageId: string } | null;
  showTimestamp?: boolean;
  timestamp: string;
}) {
  const label = formatTurnSetupLabel(items);
  const detail = turnSetupDetail(items);
  const tooltipText = [label, detail].filter(Boolean).join(" · ");
  const showSetup = items.length > 0;

  if (!showSetup) {
    return showTimestamp ? (
      <TranscriptTimestamp messageLink={messageLink} timestamp={timestamp} />
    ) : null;
  }

  return (
    <div
      className="flex items-center gap-1.5 text-muted-foreground/80"
      data-testid="transcript-turn-setup"
    >
      <span className="inline-flex shrink-0 items-center justify-center rounded-sm text-muted-foreground/70">
        <CheckCheck className="h-3.5 w-3.5" />
        <span className="sr-only">{tooltipText}</span>
      </span>
      {showTimestamp ? (
        <TranscriptTimestamp messageLink={messageLink} timestamp={timestamp} />
      ) : null}
    </div>
  );
}

function getTranscriptMessageLink(
  item: Extract<TranscriptItem, { type: "message" }>,
) {
  if (!item.channelId || !item.messageId) return null;
  return {
    channelId: item.channelId,
    messageId: item.messageId,
  };
}

function TranscriptItemRow({
  agentAvatarUrl,
  agentName,
  agentPubkey,
  item,
  profiles,
}: AgentTranscriptIdentityProps & {
  item: TranscriptItem;
  profiles?: UserProfileLookup;
}) {
  return (
    <div key={item.id}>
      {SHOW_TRANSCRIPT_ACP_SOURCE && item.acpSource ? (
        <TranscriptAcpSourceBadge source={item.acpSource} />
      ) : null}
      <TranscriptItemView
        agentAvatarUrl={agentAvatarUrl}
        agentName={agentName}
        agentPubkey={agentPubkey}
        item={item}
        profiles={profiles}
      />
    </div>
  );
}

function TurnSetupStatus({
  items,
}: {
  items: Extract<TranscriptItem, { type: "lifecycle" }>[];
}) {
  const timestamp = turnSetupTimestamp(items);
  if (items.length === 0 || !timestamp) {
    return null;
  }

  return (
    <div
      className="rounded-md px-2"
      title={formatTranscriptTimestampTitle(timestamp)}
    >
      <TurnSetupFooter
        items={items}
        showTimestamp={false}
        timestamp={timestamp}
      />
    </div>
  );
}

const TranscriptItemView = React.memo(function TranscriptItemView({
  agentAvatarUrl,
  agentName,
  agentPubkey,
  item,
  profiles,
}: AgentTranscriptIdentityProps & {
  item: TranscriptItem;
  profiles?: UserProfileLookup;
}) {
  return (
    <TranscriptActivityItem
      agentAvatarUrl={agentAvatarUrl}
      agentName={agentName}
      agentPubkey={agentPubkey}
      item={item}
      profiles={profiles}
    />
  );
});
