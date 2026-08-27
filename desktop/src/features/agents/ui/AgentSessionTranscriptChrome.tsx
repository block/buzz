import * as React from "react";
import { CheckCheck, Clock, Radio } from "lucide-react";

import type { UserProfileLookup } from "@/features/profile/lib/identity";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Toggle } from "@/shared/ui/toggle";
import type { PromptSection, TranscriptItem } from "./agentSessionTypes";
import { PromptSectionList as PromptContextSections } from "./PromptSectionAccordion";
import { useAgentSessionTranscriptVariant } from "./agentSessionTranscriptContext";
import {
  formatTurnSetupLabel,
  turnSetupDetail,
  turnSetupTimestamp,
} from "./agentSessionTranscriptGrouping";
import { formatTranscriptTimestampTitle } from "./agentSessionUtils";
import { ConversationDivider } from "./activityRenderClasses/ConversationDivider";
import { TranscriptTimestamp } from "./activityRenderClasses/TranscriptTimestamp";
import { UserMessageBubble } from "./activityRenderClasses/UserMessageBubble";

const TRANSCRIPT_ACP_SOURCE_STORAGE_KEY = "buzz:show-transcript-acp-source";

/**
 * Opt-in only: source pills are useful while iterating on observer parsing, but
 * they should not appear for every local dev session.
 */
export const SHOW_TRANSCRIPT_ACP_SOURCE = shouldShowTranscriptAcpSource();

function shouldShowTranscriptAcpSource() {
  // `import.meta.env` is bundler-provided; guard it so this module stays
  // importable from node-based render tests (same pattern as
  // features/onboarding/devFreshOnboarding.ts).
  const envValue = import.meta.env?.VITE_SHOW_TRANSCRIPT_ACP_SOURCE;
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

export function TranscriptAcpSourceBadge({ source }: { source: string }) {
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

export function TurnPromptBlock({
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
  const variant = useAgentSessionTranscriptVariant();
  const [contextOpen, setContextOpen] = React.useState(false);
  const contextSections = React.useMemo(
    () => [...(context?.sections ?? [])],
    [context],
  );

  return (
    <>
      <UserMessageBubble
        bubbleClassName={variant === "conversation" ? undefined : "p-2.5"}
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
        <div className="flex min-w-0 max-h-[85vh] flex-col">
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

export function getTranscriptMessageLink(
  item: Extract<TranscriptItem, { type: "message" }>,
) {
  if (!item.channelId || !item.messageId) return null;
  return {
    channelId: item.channelId,
    messageId: item.messageId,
  };
}

export function TurnSetupStatus({
  items,
}: {
  items: Extract<TranscriptItem, { type: "lifecycle" }>[];
}) {
  const variant = useAgentSessionTranscriptVariant();
  const timestamp = turnSetupTimestamp(items);
  if (items.length === 0 || !timestamp) {
    return null;
  }

  // Focus mode recedes turn setup to a quiet centered divider: the checks-icon
  // summary is ingress plumbing, not something a reader judges the turn by.
  if (variant === "conversation") {
    return (
      <ConversationDivider
        label={formatPromptSetupSummary(items) || formatTurnSetupLabel(items)}
        testId="transcript-turn-setup"
        title={formatTranscriptTimestampTitle(timestamp)}
      />
    );
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

/**
 * Horizontal rule rendered between session runs in the observer transcript.
 *
 * Three label states (based on live-frame observation, not harness affinity):
 *  - `"current"`     — most recent session observed via the live relay subscription.
 *  - `"most-recent"` — newest visible session with no matching live frames
 *                      (loaded from archive or session ended before observation).
 *  - `"earlier"`     — an older session preceding the most-recent one.
 */
export function SessionBoundaryDivider({
  labelState,
  sessionStartTimestamp,
}: {
  labelState: "current" | "most-recent" | "earlier";
  sessionStartTimestamp: string;
}) {
  const variant = useAgentSessionTranscriptVariant();
  const label =
    labelState === "current"
      ? "Latest live-observed session"
      : labelState === "most-recent"
        ? "Most recent observed session"
        : "Earlier observed session";
  const formattedDate = new Date(sessionStartTimestamp).toLocaleString();

  if (variant === "conversation") {
    return (
      <ConversationDivider
        icon={
          labelState === "current" ? (
            <Radio aria-hidden="true" className="h-3 w-3" />
          ) : (
            <Clock aria-hidden="true" className="h-3 w-3" />
          )
        }
        label={`${label} · ${formattedDate}`}
        testId="session-boundary-divider"
      />
    );
  }

  return (
    <div
      className="flex items-center gap-2 px-3 py-2"
      data-testid="session-boundary-divider"
    >
      <div className="h-px flex-1 bg-border" />
      <span className="flex items-center gap-1 text-xs text-muted-foreground">
        {labelState === "current" ? (
          <Radio aria-hidden="true" className="h-3 w-3" />
        ) : (
          <Clock aria-hidden="true" className="h-3 w-3" />
        )}
        {label}
        {" · "}
        {formattedDate}
      </span>
      <div className="h-px flex-1 bg-border" />
    </div>
  );
}
