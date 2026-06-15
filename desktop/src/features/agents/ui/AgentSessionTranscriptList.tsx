import * as React from "react";
import {
  AlertCircle,
  Brain,
  CheckCheck,
  ChevronDown,
  CircleDot,
  Radio,
  TerminalSquare,
} from "lucide-react";

import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Markdown } from "@/shared/ui/markdown";
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
import { formatTranscriptTime } from "./agentSessionUtils";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

const TRANSCRIPT_ACP_SOURCE_STORAGE_KEY = "buzz:show-transcript-acp-source";

/**
 * Opt-in only: source pills are useful while iterating on observer parsing, but
 * they should not appear for every local dev session.
 */
const SHOW_TRANSCRIPT_ACP_SOURCE = shouldShowTranscriptAcpSource();

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

type AgentTranscriptIdentityProps = {
  agentAvatarUrl: string | null;
  agentName: string;
  agentPubkey: string;
};

export function AgentSessionTranscriptList({
  agentAvatarUrl,
  agentName,
  agentPubkey,
  compact = false,
  emptyDescription,
  items,
  profiles,
}: AgentTranscriptIdentityProps & {
  compact?: boolean;
  emptyDescription: string;
  isWorking?: boolean;
  items: TranscriptItem[];
  profiles?: UserProfileLookup;
}) {
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
        <Radio className="mx-auto h-5 w-5 text-muted-foreground" />
        <p className="mt-3 text-sm font-medium">No ACP activity yet</p>
        <p className="mt-1 text-sm text-muted-foreground">{emptyDescription}</p>
      </div>
    );
  }

  return (
    <div className="w-full">
      <div
        aria-label="Live ACP transcript"
        aria-live="polite"
        className={cn("w-full", compact ? "py-0.5" : "py-1")}
        role="log"
      >
        {displayBlocks.map((block) => (
          <TranscriptDisplayBlockView
            agentAvatarUrl={agentAvatarUrl}
            agentName={agentName}
            agentPubkey={agentPubkey}
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
  compact,
  profiles,
}: AgentTranscriptIdentityProps & {
  block: TranscriptDisplayBlock;
  compact: boolean;
  profiles?: UserProfileLookup;
}) {
  if (block.kind === "single") {
    if (block.item.type === "tool") {
      return (
        <div className={cn("first:mt-0", compact ? "mt-3" : "mt-3.5")}>
          <ToolCallGroup compact={compact} items={[block.item]} />
        </div>
      );
    }

    return (
      <TranscriptItemRow
        agentAvatarUrl={agentAvatarUrl}
        agentName={agentName}
        agentPubkey={agentPubkey}
        compact={compact}
        item={block.item}
        profiles={profiles}
      />
    );
  }

  return (
    <div
      className={cn("first:mt-0", compact ? "mt-2.5" : "mt-3.5")}
      data-testid="transcript-turn-group"
      data-turn-id={block.turnId}
    >
      {block.segments.map((segment, index) => (
        <div
          className={getTurnSegmentSpacing(
            block.segments[index - 1],
            segment,
            compact,
          )}
          key={getTurnSegmentKey(block.turnId, segment)}
        >
          <TranscriptTurnSegmentView
            agentAvatarUrl={agentAvatarUrl}
            agentName={agentName}
            agentPubkey={agentPubkey}
            compact={compact}
            profiles={profiles}
            segment={segment}
          />
        </div>
      ))}
    </div>
  );
}

function getTurnSegmentSpacing(
  previous: TranscriptTurnSegment | undefined,
  segment: TranscriptTurnSegment,
  compact: boolean,
): string | undefined {
  if (!previous) {
    return undefined;
  }

  const involvesTool =
    previous.kind === "tool_group" || segment.kind === "tool_group";
  if (involvesTool) {
    return compact ? "mt-3" : "mt-3.5";
  }

  return compact ? "mt-2" : "mt-2.5";
}

function getTurnSegmentKey(turnId: string, segment: TranscriptTurnSegment) {
  if (segment.kind === "setup") {
    return `turn:${turnId}:setup`;
  }
  if (segment.kind === "prompt") {
    return `turn:${turnId}:prompt`;
  }
  if (segment.kind === "tool_group") {
    return `turn:${turnId}:tools:${segment.items.map((item) => item.id).join("+")}`;
  }
  return segment.item.id;
}

function TranscriptTurnSegmentView({
  agentAvatarUrl,
  agentName,
  agentPubkey,
  compact,
  profiles,
  segment,
}: AgentTranscriptIdentityProps & {
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

  if (segment.kind === "tool_group") {
    return <ToolCallGroup compact={compact} items={segment.items} />;
  }

  return (
    <TranscriptItemRow
      agentAvatarUrl={agentAvatarUrl}
      agentName={agentName}
      agentPubkey={agentPubkey}
      compact={compact}
      embedded
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
      className="flex flex-row items-start justify-end"
      data-role="user-message"
      data-testid="transcript-user-message"
    >
      <UserAvatar
        avatarUrl={authorProfile?.avatarUrl ?? null}
        className="order-last ml-2 mt-1 shrink-0"
        displayName={authorLabel}
        size="xs"
      />
      <div className="group relative flex max-w-[85%] min-w-0 flex-col items-end gap-1">
        <div
          className={cn(
            "w-full min-w-0 rounded-2xl bg-muted p-3 text-sm leading-relaxed text-foreground",
            compact && "p-2.5",
          )}
        >
          <p className="whitespace-pre-wrap wrap-break-word">{text}</p>
          {contextOpen && context ? (
            <PromptContextSections sections={context.sections} setup={setup} />
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

function PromptContextSections({
  sections,
  setup,
}: {
  sections: PromptSection[];
  setup: Extract<TranscriptItem, { type: "lifecycle" }>[];
}) {
  return (
    <div
      className="mt-2 space-y-2 border-t border-border/40 pt-2"
      data-testid="transcript-prompt-context-sections"
    >
      <PromptSetupSummary items={setup} />
      {sections.map((section) => (
        <details
          className="group/section"
          key={`${section.title}:${section.body.slice(0, 48)}`}
        >
          <summary className="inline-flex max-w-full cursor-pointer list-none items-center gap-1.5 text-xs font-medium text-foreground/80">
            <span className="truncate">{section.title}</span>
            <ChevronDown className="h-3.5 w-3.5 shrink-0 text-muted-foreground transition-transform group-open/section:rotate-180" />
          </summary>
          <pre className="mt-1.5 max-h-56 overflow-auto whitespace-pre-wrap wrap-break-word rounded-md bg-background/40 px-2 py-1.5 font-mono text-xs leading-5 text-muted-foreground">
            {section.body.trim() || "No metadata."}
          </pre>
        </details>
      ))}
    </div>
  );
}

function PromptSetupSummary({
  items,
}: {
  items: Extract<TranscriptItem, { type: "lifecycle" }>[];
}) {
  const label = formatTurnSetupLabel(items);
  const detail = turnSetupDetail(items);
  const setupText = [label, detail].filter(Boolean).join(" · ");

  if (!setupText) {
    return null;
  }

  return (
    <p
      className="text-xs leading-5 text-muted-foreground"
      data-testid="transcript-prompt-setup-summary"
    >
      {setupText}
    </p>
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

  const contextToggle = showContext ? (
    <Toggle
      aria-label={`${contextOpen ? "Hide" : "Show"} prompt context`}
      data-testid="transcript-prompt-context-toggle"
      className="data-[state=on]:bg-primary/10 data-[state=on]:text-primary dark:data-[state=on]:bg-primary/15"
      onPressedChange={onContextOpenChange}
      pressed={contextOpen}
      size="xs"
      variant="ghost"
    >
      {showSetup ? <CheckCheck aria-hidden="true" /> : null}
      Context
    </Toggle>
  ) : null;

  return (
    <div
      className="flex items-center gap-1.5 text-muted-foreground/80"
      data-testid="transcript-turn-setup"
    >
      {showContext && showSetup ? (
        <Tooltip>
          <TooltipTrigger asChild>{contextToggle}</TooltipTrigger>
          <TooltipContent side="top">
            <p>{tooltipText}</p>
          </TooltipContent>
        </Tooltip>
      ) : null}
      {!showContext && showSetup ? (
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
      {showContext && !showSetup ? contextToggle : null}
      <TranscriptTimestamp timestamp={timestamp} />
    </div>
  );
}

function ToolCallGroup({
  compact,
  items,
}: {
  compact: boolean;
  items: Extract<TranscriptItem, { type: "tool" }>[];
}) {
  return (
    <div className="flex flex-col gap-0" data-testid="transcript-tool-group">
      {items.map((item) => (
        <ToolItem compact={compact} grouped key={item.id} item={item} />
      ))}
    </div>
  );
}

function TranscriptItemRow({
  agentAvatarUrl,
  agentName,
  agentPubkey,
  compact,
  embedded = false,
  item,
  profiles,
}: AgentTranscriptIdentityProps & {
  compact: boolean;
  embedded?: boolean;
  item: TranscriptItem;
  profiles?: UserProfileLookup;
}) {
  return (
    <div
      className={cn(
        !embedded && "first:mt-0",
        !embedded && (compact ? "mt-2" : "mt-2.5"),
        !embedded && getItemSpacingClass(item),
      )}
      key={item.id}
    >
      {SHOW_TRANSCRIPT_ACP_SOURCE && item.acpSource ? (
        <TranscriptAcpSourceBadge source={item.acpSource} />
      ) : null}
      <TranscriptItemView
        agentAvatarUrl={agentAvatarUrl}
        agentName={agentName}
        agentPubkey={agentPubkey}
        compact={compact}
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
    <div className={cn("rounded-md px-2 py-1.5", compact ? "mt-1.5" : "mt-2")}>
      <TurnSetupFooter items={items} timestamp={timestamp} />
    </div>
  );
}

function getItemSpacingClass(item: TranscriptItem) {
  if (item.type === "lifecycle") {
    return "mt-1.5 first:mt-0";
  }
  if (item.type === "metadata" || item.type === "thought") {
    return "mt-1.5 first:mt-0";
  }
  return undefined;
}

const TranscriptItemView = React.memo(function TranscriptItemView({
  agentAvatarUrl,
  agentName,
  agentPubkey,
  compact,
  item,
  profiles,
}: AgentTranscriptIdentityProps & {
  compact: boolean;
  item: TranscriptItem;
  profiles?: UserProfileLookup;
}) {
  if (item.type === "message") {
    return (
      <MessageItem
        agentAvatarUrl={agentAvatarUrl}
        agentName={agentName}
        agentPubkey={agentPubkey}
        compact={compact}
        item={item}
        profiles={profiles}
      />
    );
  }
  if (item.type === "tool") {
    return <ToolItem compact={compact} item={item} />;
  }
  if (item.type === "thought") {
    return <ThoughtItem compact={compact} item={item} />;
  }
  if (item.type === "metadata") {
    return <MetadataItem compact={compact} item={item} />;
  }
  return <LifecycleItem item={item} />;
});

function MessageItem({
  agentAvatarUrl,
  agentName,
  agentPubkey,
  compact,
  item,
  profiles,
}: AgentTranscriptIdentityProps & {
  compact: boolean;
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
  const agentProfile = profiles?.[normalizePubkey(agentPubkey)] ?? null;
  const assistantLabel = resolveUserLabel({
    pubkey: agentPubkey,
    fallbackName: agentName,
    profiles,
    preferResolvedSelfLabel: true,
  });
  const assistantAvatarUrl = agentProfile?.avatarUrl ?? agentAvatarUrl;

  return (
    <div
      className={cn(
        "flex animate-in fade-in duration-200 motion-reduce:animate-none",
        isAssistant ? "flex-row" : "flex-row items-start justify-end",
        isAssistant
          ? compact
            ? "px-0 py-0"
            : "px-1 py-0"
          : compact
            ? "px-0 py-0.5"
            : "px-1 py-1",
      )}
      data-role={isAssistant ? "assistant-message" : "user-message"}
      data-testid={
        isAssistant ? "transcript-assistant-message" : "transcript-user-message"
      }
    >
      {!isAssistant ? (
        <UserAvatar
          avatarUrl={authorProfile?.avatarUrl ?? null}
          className="order-last ml-2 mt-1 shrink-0"
          displayName={authorLabel}
          size="xs"
        />
      ) : null}
      <div
        className={cn(
          "group relative flex min-w-0 flex-col gap-1",
          isAssistant ? "w-full items-start" : "max-w-[85%] items-end",
        )}
      >
        {isAssistant ? (
          <div className="mb-0.5 flex items-center gap-1.5 text-xs">
            <UserAvatar
              avatarUrl={assistantAvatarUrl}
              className="shrink-0"
              displayName={assistantLabel}
              size="xs"
              testId="transcript-assistant-avatar"
            />
            <span className="text-xs font-semibold text-foreground">
              {assistantLabel}
            </span>
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
              <p className="whitespace-pre-wrap wrap-break-word">{text}</p>
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
  item,
}: {
  compact: boolean;
  item: Extract<TranscriptItem, { type: "thought" }>;
}) {
  return (
    <details
      className={cn(
        "group not-prose w-full rounded-md border border-transparent",
        compact ? "px-0" : "px-1",
      )}
      data-testid="transcript-thought-item"
    >
      <summary className="inline-flex max-w-full cursor-pointer list-none items-center gap-1.5 py-px text-muted-foreground">
        <Brain className="h-4 w-4" />
        <span className="truncate text-sm font-medium">{item.title}</span>
        <TranscriptTimestamp timestamp={item.timestamp} />
        <ChevronDown className="h-3.5 w-3.5 shrink-0 transition-transform group-open:rotate-180" />
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
        <span className="shrink-0 text-xs text-muted-foreground/70">
          {item.sections.length} section{item.sections.length === 1 ? "" : "s"}
        </span>
        <TranscriptTimestamp timestamp={item.timestamp} />
        <ChevronDown className="h-3.5 w-3.5 shrink-0 transition-transform group-open:rotate-180" />
      </summary>
      <div className="space-y-3 py-2 pl-5">
        {item.sections.map((section) => (
          <details
            className="group/section"
            key={`${section.title}:${section.body.slice(0, 48)}`}
          >
            <summary className="inline-flex max-w-full cursor-pointer list-none items-center gap-1.5 text-xs font-medium text-foreground/80">
              <span className="truncate">{section.title}</span>
              <ChevronDown className="h-3.5 w-3.5 shrink-0 text-muted-foreground transition-transform group-open/section:rotate-180" />
            </summary>
            <pre className="mt-2 max-h-56 overflow-auto whitespace-pre-wrap wrap-break-word rounded-md bg-muted/50 px-3 py-2 font-mono text-xs leading-5 text-muted-foreground">
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
        <span className="shrink-0 cursor-default text-xs text-muted-foreground/60">
          {formatted}
        </span>
      </TooltipTrigger>
      <TooltipContent side="top">{fullDateTime}</TooltipContent>
    </Tooltip>
  );
}
