import * as React from "react";
import {
  AlertCircle,
  Bot,
  Brain,
  CheckCheck,
  ChevronDown,
  CircleDot,
  Loader2,
  Radio,
  TerminalSquare,
  Wrench,
} from "lucide-react";

import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { cn } from "@/shared/lib/cn";
import { Badge } from "@/shared/ui/badge";
import { Markdown } from "@/shared/ui/markdown";
import { Shimmer } from "@/shared/ui/Shimmer";
import { Toggle } from "@/shared/ui/toggle";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import type { PromptSection, TranscriptItem } from "./agentSessionTypes";
import { ToolItem } from "./AgentSessionToolItem";
import {
  buildTranscriptDisplayBlocks,
  formatTurnSetupLabel,
  turnSetupDetail,
  turnSetupTimestamp,
  type TranscriptDisplayBlock,
  type TranscriptTurnSegment,
} from "./agentSessionTranscriptGrouping";
import { buildTranscriptPresentation } from "./agentSessionTranscriptPresentation";
import { formatTranscriptTime } from "./agentSessionUtils";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

/** Dev-only: surface the observer wire label that produced each transcript row. */
const SHOW_TRANSCRIPT_ACP_SOURCE = import.meta.env.DEV;

export function AgentSessionTranscriptList({
  agentName,
  compact = false,
  emptyDescription,
  isWorking = false,
  items,
  profiles,
  showInterventionHint = false,
}: {
  agentName: string;
  compact?: boolean;
  emptyDescription: string;
  isWorking?: boolean;
  items: TranscriptItem[];
  profiles?: UserProfileLookup;
  showInterventionHint?: boolean;
}) {
  const presentation = React.useMemo(
    () => buildTranscriptPresentation(items, isWorking),
    [items, isWorking],
  );
  const displayBlocks = React.useMemo(
    () => buildTranscriptDisplayBlocks(items),
    [items],
  );

  if (items.length === 0) {
    return (
      <div
        className={cn(
          "flex flex-col items-center justify-center px-6 py-10 text-center",
          compact ? "min-h-40" : "min-h-56",
        )}
      >
        <Radio className="mx-auto h-4 w-4 text-muted-foreground" />
        <p className="mt-3 text-sm font-medium">No ACP activity yet</p>
        <p className="mt-1 text-sm text-muted-foreground">{emptyDescription}</p>
      </div>
    );
  }

  return (
    <div className="w-full">
      <TranscriptNowSummary
        agentName={agentName}
        compact={compact}
        isWorking={isWorking}
        presentation={presentation}
        showInterventionHint={showInterventionHint}
      />
      <div
        aria-label="Live ACP transcript"
        aria-live="polite"
        className={cn("w-full", compact ? "py-0.5" : "py-1")}
        role="log"
      >
        {displayBlocks.map((block) => (
          <TranscriptDisplayBlockView
            activeItemIds={presentation.activeItemIds}
            agentName={agentName}
            block={block}
            compact={compact}
            key={getDisplayBlockKey(block)}
            profiles={profiles}
          />
        ))}
      </div>
    </div>
  );
}

function TranscriptAcpSourceBadge({ source }: { source: string }) {
  return (
    <span
      className="mb-1 inline-flex max-w-full rounded border border-amber-500/25 bg-amber-500/10 px-1.5 py-0.5 font-mono text-[10px] leading-none text-amber-800 dark:text-amber-200"
      data-testid="transcript-acp-source"
      title={`ACP wire source: ${source}`}
    >
      {source}
    </span>
  );
}

function TranscriptNowSummary({
  agentName,
  compact,
  isWorking,
  presentation,
  showInterventionHint,
}: {
  agentName: string;
  compact: boolean;
  isWorking: boolean;
  presentation: ReturnType<typeof buildTranscriptPresentation>;
  showInterventionHint: boolean;
}) {
  const { counts, hasError, headline, lastUpdatedAt, state } = presentation;
  const showSummary = isWorking || hasError || itemsHaveActivity(counts);

  if (!showSummary) {
    return null;
  }

  const StateIcon = getStateIcon(state, isWorking);
  const statusLabel = getStateLabel(state, isWorking);
  const lastUpdated = lastUpdatedAt
    ? formatTranscriptTime(lastUpdatedAt)
    : null;

  return (
    <div
      className={cn(
        "sticky top-0 z-10 mb-3 rounded-lg border bg-background/95 backdrop-blur-sm",
        hasError
          ? "border-destructive/30 bg-destructive/[0.04]"
          : "border-border/70",
        compact ? "px-2.5 py-2" : "px-3 py-2.5",
      )}
      data-testid="agent-transcript-now-summary"
    >
      <div className="flex items-start gap-2">
        <span
          className={cn(
            "mt-0.5 flex h-5 w-5 shrink-0 items-center justify-center rounded-full",
            hasError
              ? "bg-destructive/10 text-destructive"
              : isWorking
                ? "bg-primary/10 text-primary"
                : "bg-muted text-muted-foreground",
          )}
        >
          <StateIcon
            className={cn(
              "h-3 w-3",
              isWorking &&
                state !== "error" &&
                state !== "idle" &&
                "animate-pulse",
            )}
          />
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5">
            <p className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
              Now
            </p>
            <span className="text-[11px] text-muted-foreground/70">·</span>
            <p className="text-[11px] text-muted-foreground">{agentName}</p>
            {lastUpdated ? (
              <>
                <span className="text-[11px] text-muted-foreground/70">·</span>
                <p className="text-[11px] text-muted-foreground/70">
                  {lastUpdated}
                </p>
              </>
            ) : null}
          </div>
          <p
            className={cn(
              "mt-0.5 text-sm font-medium leading-snug",
              hasError ? "text-destructive" : "text-foreground",
            )}
          >
            {isWorking && state !== "idle" && state !== "error" ? (
              <Shimmer>{headline}</Shimmer>
            ) : (
              headline
            )}
          </p>
          <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
            <Badge
              className="h-5 px-1.5 text-[10px] font-normal"
              variant={
                hasError ? "destructive" : isWorking ? "default" : "secondary"
              }
            >
              {statusLabel}
            </Badge>
            {counts.tools > 0 ? (
              <ActivityCountBadge
                count={counts.tools}
                label="tool"
                tone={counts.toolErrors > 0 ? "error" : "default"}
              />
            ) : null}
            {counts.thoughts > 0 ? (
              <ActivityCountBadge count={counts.thoughts} label="thought" />
            ) : null}
            {counts.messages > 0 ? (
              <ActivityCountBadge count={counts.messages} label="message" />
            ) : null}
          </div>
          {showInterventionHint && isWorking ? (
            <p className="mt-2 text-[11px] leading-relaxed text-muted-foreground">
              Use <span className="font-medium text-foreground">Stop</span>{" "}
              above to interrupt this turn without stopping the agent process.
            </p>
          ) : null}
        </div>
      </div>
    </div>
  );
}

function ActivityCountBadge({
  count,
  label,
  tone = "default",
}: {
  count: number;
  label: string;
  tone?: "default" | "error";
}) {
  return (
    <Badge
      className="h-5 px-1.5 text-[10px] font-normal"
      variant={tone === "error" ? "destructive" : "outline"}
    >
      {count} {label}
      {count === 1 ? "" : "s"}
    </Badge>
  );
}

function itemsHaveActivity(
  counts: ReturnType<typeof buildTranscriptPresentation>["counts"],
) {
  return (
    counts.tools > 0 ||
    counts.thoughts > 0 ||
    counts.messages > 0 ||
    counts.lifecycle > 0
  );
}

function getStateIcon(
  state: ReturnType<typeof buildTranscriptPresentation>["state"],
  isWorking: boolean,
) {
  if (state === "error") {
    return AlertCircle;
  }
  if (!isWorking) {
    return CircleDot;
  }
  switch (state) {
    case "tool_running":
      return Wrench;
    case "thinking":
      return Brain;
    case "responding":
      return Bot;
    default:
      return Loader2;
  }
}

function getStateLabel(
  state: ReturnType<typeof buildTranscriptPresentation>["state"],
  isWorking: boolean,
) {
  if (state === "error") {
    return "Error";
  }
  if (!isWorking) {
    return "Idle";
  }
  switch (state) {
    case "tool_running":
      return "Running tool";
    case "thinking":
      return "Thinking";
    case "responding":
      return "Responding";
    default:
      return "Working";
  }
}

function getDisplayBlockKey(block: TranscriptDisplayBlock) {
  if (block.kind === "single") {
    return block.item.id;
  }
  return `turn:${block.turnId}`;
}

function TranscriptDisplayBlockView({
  activeItemIds,
  agentName,
  block,
  compact,
  profiles,
}: {
  activeItemIds: ReadonlySet<string>;
  agentName: string;
  block: TranscriptDisplayBlock;
  compact: boolean;
  profiles?: UserProfileLookup;
}) {
  if (block.kind === "single") {
    return (
      <TranscriptItemRow
        activeItemIds={activeItemIds}
        agentName={agentName}
        compact={compact}
        item={block.item}
        profiles={profiles}
      />
    );
  }

  return (
    <div
      className={cn("first:mt-0", compact ? "mt-2.5" : "mt-4")}
      data-testid="transcript-turn-group"
      data-turn-id={block.turnId}
    >
      {block.segments.map((segment) => (
        <TranscriptTurnSegmentView
          activeItemIds={activeItemIds}
          agentName={agentName}
          compact={compact}
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
  return segment.item.id;
}

function TranscriptTurnSegmentView({
  activeItemIds,
  agentName,
  compact,
  profiles,
  segment,
}: {
  activeItemIds: ReadonlySet<string>;
  agentName: string;
  compact: boolean;
  profiles?: UserProfileLookup;
  segment: TranscriptTurnSegment;
}) {
  if (segment.kind === "prompt") {
    return (
      <TurnPromptBlock
        compact={compact}
        context={segment.context}
        profiles={profiles}
        setup={segment.setup}
        user={segment.user}
      />
    );
  }

  if (segment.kind === "setup") {
    return <TurnSetupStatus compact={compact} items={segment.items} />;
  }

  return (
    <TranscriptItemRow
      activeItemIds={activeItemIds}
      agentName={agentName}
      compact={compact}
      item={segment.item}
      profiles={profiles}
    />
  );
}

function TurnPromptBlock({
  compact,
  context,
  profiles,
  setup,
  user,
}: {
  compact: boolean;
  context: Extract<TranscriptItem, { type: "metadata" }> | null;
  profiles?: UserProfileLookup;
  setup: Extract<TranscriptItem, { type: "lifecycle" }>[];
  user: Extract<TranscriptItem, { type: "message" }>;
}) {
  return (
    <div
      className={cn("first:mt-0", compact ? "mt-2.5" : "mt-4")}
      data-testid="transcript-prompt-bundle"
    >
      {SHOW_TRANSCRIPT_ACP_SOURCE ? (
        <div className="mb-1 flex flex-wrap gap-1">
          <TranscriptAcpSourceBadge source="session/prompt:user" />
          {context ? (
            <TranscriptAcpSourceBadge source="session/prompt:context" />
          ) : null}
        </div>
      ) : null}
      <PromptUserMessage
        compact={compact}
        context={context}
        item={user}
        profiles={profiles}
        setup={setup}
      />
    </div>
  );
}

function PromptUserMessage({
  compact,
  context = null,
  item,
  profiles,
  setup = [],
}: {
  compact: boolean;
  context?: Extract<TranscriptItem, { type: "metadata" }> | null;
  item: Extract<TranscriptItem, { type: "message" }>;
  profiles?: UserProfileLookup;
  setup?: Extract<TranscriptItem, { type: "lifecycle" }>[];
}) {
  const [contextOpen, setContextOpen] = React.useState(false);
  const text = item.text.trim();
  const authorProfile = item.authorPubkey
    ? profiles?.[item.authorPubkey.toLowerCase()]
    : null;
  const authorLabel = item.authorPubkey
    ? resolveUserLabel({
        pubkey: item.authorPubkey,
        fallbackName: item.title,
        profiles,
      })
    : item.title || "User";

  return (
    <div
      className="flex flex-row"
      data-role="user-message"
      data-testid="transcript-user-message"
    >
      <UserAvatar
        avatarUrl={authorProfile?.avatarUrl ?? null}
        className="mr-2 mt-1 h-5 w-5 shrink-0 text-[8px]"
        displayName={authorLabel}
        size="xs"
      />
      <div className="group relative min-w-0 max-w-[85%] flex flex-col items-start gap-1">
        <div
          className={cn(
            "w-full min-w-0 rounded-2xl bg-muted p-3 text-sm leading-relaxed text-foreground",
            compact && "p-2.5",
          )}
        >
          <p className="whitespace-pre-wrap break-words">{text}</p>
          {contextOpen && context ? (
            <PromptContextSections sections={context.sections} />
          ) : null}
        </div>
        <TurnSetupFooter
          context={context}
          contextOpen={contextOpen}
          items={setup}
          onContextOpenChange={setContextOpen}
          timestamp={item.timestamp}
        />
      </div>
    </div>
  );
}

function PromptContextSections({ sections }: { sections: PromptSection[] }) {
  return (
    <div
      className="mt-2 space-y-2 border-t border-border/40 pt-2"
      data-testid="transcript-prompt-context-sections"
    >
      {sections.map((section) => (
        <details
          className="group/section"
          key={`${section.title}:${section.body.slice(0, 48)}`}
        >
          <summary className="inline-flex max-w-full cursor-pointer list-none items-center gap-1.5 text-xs font-medium text-foreground/80">
            <span className="truncate">{section.title}</span>
            <ChevronDown className="h-3.5 w-3.5 shrink-0 text-muted-foreground transition-transform group-open/section:rotate-180" />
          </summary>
          <pre className="mt-1.5 max-h-56 overflow-auto whitespace-pre-wrap break-words rounded-md bg-background/40 px-2 py-1.5 font-mono text-[11px] leading-5 text-muted-foreground">
            {section.body.trim() || "No metadata."}
          </pre>
        </details>
      ))}
    </div>
  );
}

function TurnSetupFooter({
  context = null,
  contextOpen = false,
  items,
  onContextOpenChange,
  timestamp,
}: {
  context?: Extract<TranscriptItem, { type: "metadata" }> | null;
  contextOpen?: boolean;
  items: Extract<TranscriptItem, { type: "lifecycle" }>[];
  onContextOpenChange?: (open: boolean) => void;
  timestamp: string;
}) {
  const label = formatTurnSetupLabel(items);
  const detail = turnSetupDetail(items);
  const tooltipText = [label, detail].filter(Boolean).join(" · ");
  const showSetup = items.length > 0;
  const showContext = context != null && context.sections.length > 0;

  if (!showSetup && !showContext) {
    return <TranscriptTimestamp timestamp={timestamp} />;
  }

  return (
    <div
      className="flex items-center gap-1.5 text-muted-foreground/80"
      data-testid="transcript-turn-setup"
    >
      {showSetup ? (
        <Tooltip>
          <TooltipTrigger asChild>
            <button
              className="inline-flex shrink-0 items-center justify-center rounded-sm text-muted-foreground/70 transition-colors hover:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
              type="button"
            >
              <CheckCheck className="h-3.5 w-3.5" />
              <span className="sr-only">{tooltipText}</span>
            </button>
          </TooltipTrigger>
          <TooltipContent side="top">
            <p>{tooltipText}</p>
          </TooltipContent>
        </Tooltip>
      ) : null}
      {showContext ? (
        <Toggle
          aria-label={`${contextOpen ? "Hide" : "Show"} prompt context`}
          className="h-5 min-h-0 min-w-0 gap-1 rounded-md px-1.5 text-[10px] font-medium data-[state=on]:bg-muted data-[state=on]:text-foreground"
          data-testid="transcript-prompt-context-toggle"
          onPressedChange={onContextOpenChange}
          pressed={contextOpen}
          size="sm"
          variant="outline"
        >
          Context
          <span className="text-muted-foreground/70">
            {context.sections.length}
          </span>
        </Toggle>
      ) : null}
      <TranscriptTimestamp timestamp={timestamp} />
    </div>
  );
}

function TranscriptItemRow({
  activeItemIds,
  agentName,
  compact,
  item,
  profiles,
}: {
  activeItemIds: ReadonlySet<string>;
  agentName: string;
  compact: boolean;
  item: TranscriptItem;
  profiles?: UserProfileLookup;
}) {
  return (
    <div
      className={cn(
        "first:mt-0",
        compact ? "mt-2.5" : "mt-4",
        getItemSpacingClass(item),
      )}
      key={item.id}
    >
      {SHOW_TRANSCRIPT_ACP_SOURCE && item.acpSource ? (
        <TranscriptAcpSourceBadge source={item.acpSource} />
      ) : null}
      <TranscriptItemView
        agentName={agentName}
        compact={compact}
        isActive={activeItemIds.has(item.id)}
        item={item}
        profiles={profiles}
      />
    </div>
  );
}

function TurnSetupStatus({
  compact,
  items,
}: {
  compact: boolean;
  items: Extract<TranscriptItem, { type: "lifecycle" }>[];
}) {
  const timestamp = turnSetupTimestamp(items);
  if (items.length === 0 || !timestamp) {
    return null;
  }

  return (
    <div className={cn("rounded-md px-2 py-1.5", compact ? "mt-2" : "mt-2.5")}>
      <TurnSetupFooter items={items} timestamp={timestamp} />
    </div>
  );
}

function getItemSpacingClass(item: TranscriptItem) {
  if (item.type === "lifecycle") {
    return "mt-2 first:mt-0";
  }
  if (item.type === "metadata" || item.type === "thought") {
    return "mt-2 first:mt-0";
  }
  return undefined;
}

const TranscriptItemView = React.memo(function TranscriptItemView({
  agentName,
  compact,
  isActive,
  item,
  profiles,
}: {
  agentName: string;
  compact: boolean;
  isActive: boolean;
  item: TranscriptItem;
  profiles?: UserProfileLookup;
}) {
  if (item.type === "message") {
    return (
      <MessageItem
        agentName={agentName}
        compact={compact}
        isActive={isActive}
        item={item}
        profiles={profiles}
      />
    );
  }
  if (item.type === "tool") {
    return <ToolItem compact={compact} isActive={isActive} item={item} />;
  }
  if (item.type === "thought") {
    return <ThoughtItem compact={compact} isActive={isActive} item={item} />;
  }
  if (item.type === "metadata") {
    return <MetadataItem compact={compact} item={item} />;
  }
  return <LifecycleItem item={item} />;
});

function MessageItem({
  agentName,
  compact,
  isActive,
  item,
  profiles,
}: {
  agentName: string;
  compact: boolean;
  isActive: boolean;
  item: Extract<TranscriptItem, { type: "message" }>;
  profiles?: UserProfileLookup;
}) {
  const isAssistant = item.role === "assistant";
  const text = item.text.trim();
  const authorProfile = item.authorPubkey
    ? profiles?.[item.authorPubkey.toLowerCase()]
    : null;
  const authorLabel = item.authorPubkey
    ? resolveUserLabel({
        pubkey: item.authorPubkey,
        fallbackName: item.title,
        profiles,
      })
    : item.title || "User";

  return (
    <div
      className={cn(
        "flex flex-row animate-in fade-in duration-200 motion-reduce:animate-none",
        compact ? "px-0 py-0.5" : "px-1 py-1",
        isAssistant &&
          isActive &&
          "rounded-lg border border-primary/15 bg-primary/[0.03] px-2 py-1.5",
      )}
      data-role={isAssistant ? "assistant-message" : "user-message"}
      data-testid={
        isAssistant ? "transcript-assistant-message" : "transcript-user-message"
      }
    >
      {!isAssistant ? (
        <UserAvatar
          avatarUrl={authorProfile?.avatarUrl ?? null}
          className="mr-2 mt-1 h-5 w-5 shrink-0 text-[8px]"
          displayName={authorLabel}
          size="xs"
        />
      ) : null}
      <div
        className={cn(
          "group relative min-w-0 flex flex-col items-start gap-1",
          isAssistant ? "w-full" : "max-w-[85%]",
        )}
      >
        {isAssistant ? (
          <div className="mb-0.5 flex items-center gap-1 text-xs">
            <span className="flex h-5 w-5 items-center justify-center">
              <Bot className="h-4 w-4 text-muted-foreground" />
            </span>
            <span className="font-normal text-foreground">{agentName}</span>
            {isActive ? (
              <Badge
                className="h-4 gap-0.5 px-1 text-[9px] font-normal"
                variant="default"
              >
                <CircleDot className="h-2 w-2" />
                Live
              </Badge>
            ) : null}
            <TranscriptTimestamp timestamp={item.timestamp} />
          </div>
        ) : null}
        <div
          className={cn(
            "w-full min-w-0 text-sm leading-relaxed",
            !isAssistant && "rounded-2xl bg-muted p-3 text-foreground",
          )}
        >
          {isAssistant ? (
            <Markdown compact content={text || " "} />
          ) : (
            <>
              <p className="whitespace-pre-wrap break-words">{text}</p>
              <TranscriptTimestamp timestamp={item.timestamp} />
            </>
          )}
        </div>
      </div>
    </div>
  );
}

function ThoughtItem({
  compact,
  isActive,
  item,
}: {
  compact: boolean;
  isActive: boolean;
  item: Extract<TranscriptItem, { type: "thought" }>;
}) {
  return (
    <details
      className={cn(
        "group not-prose w-full rounded-md border border-transparent",
        compact ? "px-0" : "px-1",
        isActive && "border-primary/15 bg-primary/[0.03] px-2 py-1",
      )}
      data-testid="transcript-thought-item"
    >
      <summary className="inline-flex max-w-full cursor-pointer list-none items-center gap-1.5 py-px text-muted-foreground">
        <Brain className={cn("h-4 w-4", isActive && "text-primary")} />
        <span className="truncate text-sm font-medium">{item.title}</span>
        {isActive ? (
          <Badge
            className="h-4 gap-0.5 px-1 text-[9px] font-normal"
            variant="default"
          >
            <CircleDot className="h-2 w-2" />
            Live
          </Badge>
        ) : null}
        <TranscriptTimestamp timestamp={item.timestamp} />
        <ChevronDown className="h-4 w-4 shrink-0 transition-transform group-open:rotate-180" />
      </summary>
      <div className="py-2 pl-5 text-sm leading-6 text-muted-foreground">
        <Markdown compact content={item.text.trim() || " "} />
      </div>
    </details>
  );
}

function MetadataItem({
  compact,
  embedded = false,
  item,
}: {
  compact: boolean;
  embedded?: boolean;
  item: Extract<TranscriptItem, { type: "metadata" }>;
}) {
  return (
    <details
      className={cn(
        "group not-prose w-full",
        embedded
          ? compact
            ? "px-2 py-1"
            : "px-2.5 py-1.5"
          : cn(
              "rounded-md border border-border/50 bg-muted/20",
              compact ? "px-2 py-1" : "px-2 py-1.5",
            ),
      )}
      data-testid="transcript-metadata-item"
    >
      <summary className="inline-flex max-w-full cursor-pointer list-none items-center gap-1.5 py-px text-muted-foreground">
        <TerminalSquare className="h-3.5 w-3.5 shrink-0 opacity-70" />
        <span className="truncate text-xs font-medium">{item.title}</span>
        <span className="shrink-0 text-[10px] text-muted-foreground/70">
          {item.sections.length} section{item.sections.length === 1 ? "" : "s"}
        </span>
        <TranscriptTimestamp timestamp={item.timestamp} />
        <ChevronDown className="h-4 w-4 shrink-0 transition-transform group-open:rotate-180" />
      </summary>
      <div className="space-y-3 py-2 pl-5">
        {item.sections.map((section) => (
          <details
            className="group/section"
            key={`${section.title}:${section.body.slice(0, 48)}`}
          >
            <summary className="inline-flex max-w-full cursor-pointer list-none items-center gap-1.5 text-xs font-medium text-foreground/80">
              <span className="truncate">{section.title}</span>
              <ChevronDown className="h-4 w-4 shrink-0 text-muted-foreground transition-transform group-open/section:rotate-180" />
            </summary>
            <pre className="mt-2 max-h-56 overflow-auto whitespace-pre-wrap break-words rounded-md bg-muted/50 px-3 py-2 font-mono text-[11px] leading-5 text-muted-foreground">
              {section.body.trim() || "No metadata."}
            </pre>
          </details>
        ))}
      </div>
    </details>
  );
}

function LifecycleItem({
  item,
}: {
  item: Extract<TranscriptItem, { type: "lifecycle" }>;
}) {
  const isError = item.title.toLowerCase().includes("error");
  return (
    <div
      className={cn(
        "flex items-center justify-start gap-1.5 rounded-md px-2 py-1.5 text-left text-xs",
        isError
          ? "border border-destructive/20 bg-destructive/5 text-destructive"
          : "text-muted-foreground/80",
      )}
      data-testid="transcript-lifecycle-item"
    >
      {isError ? (
        <AlertCircle className="h-3.5 w-3.5 shrink-0" />
      ) : (
        <CircleDot className="h-3 w-3 shrink-0 opacity-50" />
      )}
      <span className="font-medium">{item.title}</span>
      {item.text ? <span className="opacity-80">· {item.text}</span> : null}
      <TranscriptTimestamp timestamp={item.timestamp} />
    </div>
  );
}

const fullDateTimeFormat = new Intl.DateTimeFormat(undefined, {
  weekday: "long",
  year: "numeric",
  month: "long",
  day: "numeric",
  hour: "numeric",
  minute: "2-digit",
  second: "2-digit",
});

function TranscriptTimestamp({ timestamp }: { timestamp: string }) {
  const formatted = formatTranscriptTime(timestamp);
  if (!formatted) return null;
  const date = new Date(timestamp);
  const fullDateTime = Number.isNaN(date.getTime())
    ? timestamp
    : fullDateTimeFormat.format(date);
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span className="shrink-0 cursor-default text-[11px] text-muted-foreground/60">
          {formatted}
        </span>
      </TooltipTrigger>
      <TooltipContent side="top">{fullDateTime}</TooltipContent>
    </Tooltip>
  );
}
