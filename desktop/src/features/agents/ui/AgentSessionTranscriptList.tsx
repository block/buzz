import * as React from "react";
import { CheckCheck, ChevronDown, Radio } from "lucide-react";

import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { cn } from "@/shared/lib/cn";
import { Markdown } from "@/shared/ui/markdown";
import { Toggle } from "@/shared/ui/toggle";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import type { PromptSection, TranscriptItem } from "./agentSessionTypes";
import { TranscriptActivityItem } from "./activityRenderClasses/TranscriptActivityItem";
import {
  ActivityRow,
  ActivityRowContent,
  ActivityRowLabel,
  splitActivityRowLabel,
} from "./activityRenderClasses/ActivityRow";
import { TranscriptTimestamp } from "./activityRenderClasses/TranscriptTimestamp";
import type { AgentTranscriptIdentityProps } from "./activityRenderClasses/types";
import {
  buildTranscriptDisplayBlocks,
  formatTurnSetupLabel,
  turnSetupDetail,
  turnSetupTimestamp,
  type TranscriptDisplayBlock,
  type TranscriptTurnSegment,
} from "./agentSessionTranscriptGrouping";

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

export function AgentSessionTranscriptList({
  agentAvatarUrl,
  agentName,
  agentPubkey,
  emptyDescription,
  items,
  profiles,
}: AgentTranscriptIdentityProps & {
  emptyDescription: string;
  items: TranscriptItem[];
  profiles?: UserProfileLookup;
}) {
  const displayBlocks = React.useMemo(
    () => buildTranscriptDisplayBlocks(items),
    [items],
  );

  if (items.length === 0) {
    return (
      <div className="flex min-h-40 flex-col items-center justify-center px-6 py-10 text-center">
        <Radio className="mx-auto h-4 w-4 text-muted-foreground" />
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
        className="flex w-full flex-col gap-2.5"
        role="log"
      >
        {displayBlocks.map((block) => (
          <div
            className="content-visibility-auto"
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
      className="flex flex-col gap-2.5"
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
  const expandsToToolItems = summary.items.every(
    (item) => item.type === "tool",
  );

  return (
    <ActivityRow
      className="flex flex-col gap-0.5"
      openToneScope="summary"
      testId="transcript-same-kind-summary"
    >
      <ToolRunSummaryLabel label={summary.label} />
      <TranscriptTimestamp timestamp={summary.timestamp} />
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

function ToolRunSummaryLabel({ label }: { label: string }) {
  const parts = splitActivityRowLabel(label);

  if (!parts) {
    return <span className="truncate text-sm font-medium">{label}</span>;
  }

  return (
    <ActivityRowLabel
      object={parts.object}
      openToneScope="summary"
      verb={parts.verb}
    />
  );
}

function TurnPromptBlock({
  context,
  profiles,
  setup,
  user,
}: {
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
        context={context}
        item={user}
        profiles={profiles}
        setup={setup}
      />
    </div>
  );
}

function PromptUserMessage({
  context = null,
  item,
  profiles,
  setup = [],
}: {
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
        <div className="w-full min-w-0 rounded-2xl bg-muted p-2.5 text-sm leading-relaxed text-foreground">
          <Markdown content={text || " "} mediaInset tight />
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
    <div className="rounded-md px-2">
      <TurnSetupFooter items={items} timestamp={timestamp} />
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
