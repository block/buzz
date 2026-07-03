import * as React from "react";
import { motion, useReducedMotion } from "motion/react";
import { CheckCheck, Radio } from "lucide-react";

import {
  useActiveAgentTurns,
  type ActiveTurnSummary,
} from "@/features/agents/activeAgentTurnsStore";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { useAnchoredScroll } from "@/features/messages/ui/useAnchoredScroll";
import { cn } from "@/shared/lib/cn";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Toggle } from "@/shared/ui/toggle";
import { FuzzyLogo } from "@/shared/ui/buzz-logo/FuzzyLogo";
import type { PromptSection, TranscriptItem } from "./agentSessionTypes";
import { TurnLivenessIndicator } from "./TurnLivenessIndicator";
import { PromptSectionList as PromptContextSections } from "./PromptSectionAccordion";
import {
  AgentSessionTranscriptVariantProvider,
  type AgentSessionTranscriptVariant,
  useAgentSessionTranscriptVariant,
} from "./agentSessionTranscriptContext";
import { useTranscriptAnimationEnabled } from "./transcriptAnimationPreference";
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

const ROW_ENTER_SPRING = {
  damping: 38,
  stiffness: 480,
  type: "spring",
} as const;
const ROW_ENTER_FROM = { opacity: 0, y: 12 } as const;
const ROW_ENTER_TO = { opacity: 1, y: 0 } as const;

/**
 * False during the mount commit, true afterwards. Children mounted with the
 * initial batch (history load) read false and skip their enter animation;
 * children appended later read true and animate in.
 */
function useHasCompletedInitialRender() {
  const ref = React.useRef(false);
  React.useEffect(() => {
    ref.current = true;
  }, []);
  return ref;
}

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
  channelId = null,
  emptyDescription,
  emptyState = "idle",
  items,
  profiles,
  contentContainerClassName,
  scrollScopeKey,
  variant = "default",
}: AgentTranscriptIdentityProps & {
  autoTail?: boolean;
  channelId?: string | null;
  emptyDescription: string;
  emptyState?: AgentSessionTranscriptEmptyState;
  items: TranscriptItem[];
  profiles?: UserProfileLookup;
  contentContainerClassName?: string;
  scrollScopeKey?: string | null;
  variant?: AgentSessionTranscriptVariant;
}) {
  const activeTurns = useActiveAgentTurns(agentPubkey);
  const isTurnLive = React.useMemo(
    () => isAgentTurnLive(activeTurns, channelId),
    [activeTurns, channelId],
  );
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

  const isCompactPreview = variant === "compactPreview";
  const animationPreferenceEnabled = useTranscriptAnimationEnabled();
  const shouldReduceMotion = useReducedMotion();
  const animationsDisabled =
    Boolean(shouldReduceMotion) || !animationPreferenceEnabled;
  // Position (layout) animations are only safe when this component owns the
  // scroll container: `layoutScroll` below tells motion to subtract our scroll
  // offset when measuring rows. When an ancestor scrolls instead (autoTail
  // off), scrolling would register as false position deltas and rows would
  // visibly spring back toward their pre-scroll position, so only the enter
  // animation runs there.
  const layoutAnimationsEnabled = !animationsDisabled && autoTail;
  const hasCompletedInitialRenderRef = useHasCompletedInitialRender();
  const hasRenderableContent =
    items.length > 0 && hasRenderableDisplayContent(displayBlocks, variant);

  const scrollContainerClassNames = cn(
    "w-full",
    autoTail ? "h-full overflow-y-auto" : null,
  );

  if (!hasRenderableContent) {
    const isLoading = emptyState === "loading" || isTurnLive;

    return (
      <div className={scrollContainerClassNames}>
        <div className="flex h-full min-h-40 flex-col items-center justify-center px-6 py-10 text-center">
          {isLoading ? (
            <FuzzyLogo
              ariaLabel="Waiting for ACP activity"
              className="mx-auto text-muted-foreground"
              fuzz={false}
              loop
            />
          ) : (
            <>
              <Radio className="mx-auto h-4 w-4 text-muted-foreground" />
              <p className="mt-3 text-sm font-medium">No ACP activity yet</p>
              <p className="mt-1 text-sm text-muted-foreground">
                {emptyDescription}
              </p>
            </>
          )}
        </div>
      </div>
    );
  }

  return (
    <motion.div
      className={scrollContainerClassNames}
      layoutScroll
      onScroll={autoTail ? anchoredScroll.onScroll : undefined}
      ref={autoTail ? scrollContainerRef : undefined}
    >
      <div
        aria-label="Live ACP transcript"
        aria-live="polite"
        className={cn(
          "flex w-full flex-col",
          isCompactPreview ? "gap-1" : "gap-4",
          autoTail && "pb-4",
          contentContainerClassName,
        )}
        ref={autoTail ? contentRef : undefined}
        role="log"
      >
        <AgentSessionTranscriptVariantProvider value={variant}>
          {displayBlocks.map((block) => {
            const blockKey = getDisplayBlockKey(block);
            return (
              <motion.div
                animate={ROW_ENTER_TO}
                data-message-id={blockKey}
                initial={
                  animationsDisabled || !hasCompletedInitialRenderRef.current
                    ? false
                    : ROW_ENTER_FROM
                }
                key={blockKey}
                layout={layoutAnimationsEnabled ? "position" : false}
                transition={ROW_ENTER_SPRING}
              >
                {/* content-visibility stays on a non-animated child: motion
                    measures the outer wrapper for layout animations, which
                    would otherwise force skipped offscreen rows to render. */}
                <div className="content-visibility-auto">
                  <TranscriptDisplayBlockView
                    agentAvatarUrl={agentAvatarUrl}
                    agentName={agentName}
                    agentPubkey={agentPubkey}
                    block={block}
                    profiles={profiles}
                  />
                </div>
              </motion.div>
            );
          })}
          {isTurnLive && !isCompactPreview ? <TurnLivenessIndicator /> : null}
        </AgentSessionTranscriptVariantProvider>
      </div>
    </motion.div>
  );
}

function isAgentTurnLive(
  activeTurns: ActiveTurnSummary[],
  channelId: string | null,
) {
  if (activeTurns.length === 0) {
    return false;
  }
  if (!channelId) {
    return true;
  }
  return activeTurns.some((turn) => turn.channelId === channelId);
}

function hasRenderableDisplayContent(
  displayBlocks: TranscriptDisplayBlock[],
  variant: AgentSessionTranscriptVariant,
) {
  if (variant !== "compactPreview") {
    return displayBlocks.length > 0;
  }

  return displayBlocks.some(hasRenderableCompactBlock);
}

function hasRenderableCompactBlock(block: TranscriptDisplayBlock) {
  if (block.kind === "single") {
    return isRenderableCompactItem(block.item);
  }

  return block.segments.some((segment) => {
    if (segment.kind === "item") {
      return isRenderableCompactItem(segment.item);
    }
    if (segment.kind === "prompt") {
      return true;
    }
    if (segment.kind === "summary") {
      return segment.summary.items.some(isRenderableCompactItem);
    }
    return false;
  });
}

function isRenderableCompactItem(item: TranscriptItem) {
  return item.renderClass !== "raw-rail" && item.renderClass !== "suppressed";
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
  const animationPreferenceEnabled = useTranscriptAnimationEnabled();
  const shouldReduceMotion = useReducedMotion();
  // Streaming tool calls land as new segments inside the current turn block
  // (the block key stays `turn:<id>`), so the list-level enter animation
  // never fires for them — each segment animates in here instead. Segments
  // present when the block mounts (history load, or the first paint of a new
  // turn — the block wrapper already animates that) skip the transition.
  const hasCompletedInitialRenderRef = useHasCompletedInitialRender();
  const animateSegmentEnter = animationPreferenceEnabled && !shouldReduceMotion;

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
        <motion.div
          animate={ROW_ENTER_TO}
          initial={
            animateSegmentEnter && hasCompletedInitialRenderRef.current
              ? ROW_ENTER_FROM
              : false
          }
          key={getTurnSegmentKey(block.turnId, segment)}
          transition={ROW_ENTER_SPRING}
        >
          <TranscriptTurnSegmentView
            agentAvatarUrl={agentAvatarUrl}
            agentName={agentName}
            agentPubkey={agentPubkey}
            profiles={profiles}
            segment={segment}
          />
        </motion.div>
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
  const [contextOpen, setContextOpen] = React.useState(false);
  const contextSections = React.useMemo(
    () => [...(systemPrompt?.sections ?? []), ...(context?.sections ?? [])],
    [context, systemPrompt],
  );

  return (
    <>
      <UserMessageBubble
        bubbleClassName="p-2.5"
        footer={
          <TurnSetupFooter
            contextOpen={contextOpen}
            hasContext={contextSections.length > 0}
            items={setup}
            messageLink={getTranscriptMessageLink(item)}
            onContextOpenChange={setContextOpen}
            timestamp={item.timestamp}
          />
        }
        item={item}
        profiles={profiles}
      />
      <PromptContextDialog
        onOpenChange={setContextOpen}
        open={contextOpen}
        sections={contextSections}
        setup={setup}
      />
    </>
  );
}

function PromptContextDialog({
  onOpenChange,
  open,
  sections,
  setup,
}: {
  onOpenChange: (open: boolean) => void;
  open: boolean;
  sections: PromptSection[];
  setup: Extract<TranscriptItem, { type: "lifecycle" }>[];
}) {
  if (!open || sections.length === 0) {
    return null;
  }

  const setupText = formatPromptSetupSummary(setup);

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="max-w-xl overflow-hidden p-0">
        <div className="flex max-h-[85vh] flex-col">
          <DialogHeader className="px-6 pb-3 pt-5 pr-14">
            <DialogTitle>Prompt context</DialogTitle>
            {setupText ? (
              <div className="flex items-center gap-1.5">
                <CheckCheck className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                <DialogDescription>{setupText}</DialogDescription>
              </div>
            ) : null}
          </DialogHeader>
          <div className="min-h-0 flex-1 overflow-y-auto px-6 pb-6 pt-2">
            <PromptContextSections sections={sections} />
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function formatPromptSetupSummary(
  items: Extract<TranscriptItem, { type: "lifecycle" }>[],
) {
  const label = formatTurnSetupLabel(items);
  const detail = turnSetupDetail(items);
  return [label, detail].filter(Boolean).join(" · ");
}

function TurnSetupFooter({
  contextOpen = false,
  hasContext = false,
  items,
  messageLink = null,
  onContextOpenChange,
  showTimestamp = true,
  timestamp,
}: {
  contextOpen?: boolean;
  hasContext?: boolean;
  items: Extract<TranscriptItem, { type: "lifecycle" }>[];
  messageLink?: { channelId: string; messageId: string } | null;
  onContextOpenChange?: (open: boolean) => void;
  showTimestamp?: boolean;
  timestamp: string;
}) {
  const label = formatTurnSetupLabel(items);
  const detail = turnSetupDetail(items);
  const tooltipText = [label, detail].filter(Boolean).join(" · ");
  const showSetup = items.length > 0;
  const showContext = hasContext && onContextOpenChange != null;

  if (!showSetup && !showContext) {
    return showTimestamp ? (
      <TranscriptTimestamp messageLink={messageLink} timestamp={timestamp} />
    ) : null;
  }

  return (
    <div
      className="flex items-center gap-1.5 text-muted-foreground/80"
      data-testid="transcript-turn-setup"
    >
      {showContext ? (
        <Toggle
          aria-label={`${contextOpen ? "Hide" : "Show"} prompt context`}
          className="data-[state=on]:bg-primary/10 data-[state=on]:text-primary dark:data-[state=on]:bg-primary/15"
          data-testid="transcript-prompt-context-toggle"
          onPressedChange={onContextOpenChange}
          pressed={contextOpen}
          size="xs"
          title={tooltipText || "Show prompt context"}
          variant="ghost"
        >
          <CheckCheck aria-hidden="true" />
        </Toggle>
      ) : (
        <span className="inline-flex shrink-0 items-center justify-center rounded-sm text-muted-foreground/70">
          <CheckCheck className="h-3.5 w-3.5" />
          <span className="sr-only">{tooltipText}</span>
        </span>
      )}
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
