import * as React from "react";
import { CheckCheck, ChevronDown, Clock3, Coins, Brain } from "lucide-react";

import { PromptContextDialog } from "@/features/agents/ui/PromptContextDialog";
import type { TranscriptItem } from "@/features/agents/ui/agentSessionTypes";
import { Markdown } from "@/shared/ui/markdown";
import { cn } from "@/shared/lib/cn";
import {
  useAgentMessageTurnMeta,
  type AgentMessageTurnMeta,
} from "@/features/messages/useAgentMessageTurnMeta";
import type { TimelineMessage } from "@/features/messages/types";

type ChromeContextValue = {
  meta: AgentMessageTurnMeta;
  expanded: boolean;
  setExpanded: React.Dispatch<React.SetStateAction<boolean>>;
  contextOpen: boolean;
  setContextOpen: React.Dispatch<React.SetStateAction<boolean>>;
  showThoughtChip: boolean;
  messageId: string;
};

const AgentMessageTurnChromeContext =
  React.createContext<ChromeContextValue | null>(null);

function useChromeContext(): ChromeContextValue | null {
  return React.useContext(AgentMessageTurnChromeContext);
}

type RootProps = {
  channelId: string | null | undefined;
  message: TimelineMessage;
  children: React.ReactNode;
};

/**
 * Owns turn meta + expand state for under-body chips and the thinking card.
 * Mount around the message column so body, expand, and chips share one join.
 */
export function AgentMessageTurnChromeRoot({
  channelId,
  message,
  children,
}: RootProps) {
  const [expanded, setExpanded] = React.useState(false);
  const [contextOpen, setContextOpen] = React.useState(false);
  const meta = useAgentMessageTurnMeta({
    agentPubkey: message.pubkey,
    channelId,
    messageId: message.id,
    createdAt: message.createdAt,
    parentId: message.parentId,
    isAgent: message.isAgent === true,
  });

  // Thought chip only when the setting is on and this turn has thought/tool content.
  const showThoughtChip = meta.enabled && meta.hasThinkingContent;
  React.useEffect(() => {
    if (!showThoughtChip && expanded) setExpanded(false);
  }, [showThoughtChip, expanded]);

  const value = React.useMemo(
    () => ({
      meta,
      expanded,
      setExpanded,
      contextOpen,
      setContextOpen,
      showThoughtChip,
      messageId: message.id,
    }),
    [meta, expanded, contextOpen, showThoughtChip, message.id],
  );

  return (
    <AgentMessageTurnChromeContext.Provider value={value}>
      {children}
    </AgentMessageTurnChromeContext.Provider>
  );
}

/**
 * Duration / Thought / tokens / prompt-context chips under the reply body.
 * Thinking expand mounts under this chip row when open.
 */
export function AgentMessageTurnFooterChrome() {
  const ctx = useChromeContext();
  if (!ctx?.meta.enabled) return null;

  const {
    meta,
    expanded,
    setExpanded,
    contextOpen,
    setContextOpen,
    showThoughtChip,
    messageId,
  } = ctx;

  // Empty chips must not render (no em dash / blank pill). Loading may show "…".
  // Absurd multi-hour spans are already null from resolveTurnDurationSeconds
  // (Browser progress mid-turn / thread-ancestor joins), so missing tokens +
  // missing duration hide both cleanly; when usage exists, tokens still show.
  const showDurationChip = Boolean(meta.durationLabel) || meta.durationLoading;
  const showTokensChip = Boolean(meta.tokensLabel) || meta.metricLoading;
  const showContextChip = meta.hasPromptContext;
  const durationText =
    meta.durationLabel ?? (meta.durationLoading ? "…" : null);
  const tokensText = meta.tokensLabel ?? (meta.metricLoading ? "…" : null);

  const setup = meta.promptSetupItems.filter(
    (item): item is Extract<TranscriptItem, { type: "lifecycle" }> =>
      item.type === "lifecycle",
  );

  const hasAnyChip =
    showDurationChip || showTokensChip || showContextChip || showThoughtChip;
  if (!hasAnyChip && !expanded && !contextOpen) return null;

  return (
    <div
      className="mt-1.5"
      data-testid={`agent-message-turn-footer-${messageId}`}
    >
      {hasAnyChip ? (
        <span
          className="inline-flex shrink-0 flex-wrap items-center gap-1"
          data-testid={`agent-message-turn-chrome-${messageId}`}
        >
          {showDurationChip && durationText ? (
            <span
              className="inline-flex items-center gap-1 rounded-full border border-border/60 bg-background/60 px-2 py-0.5 text-2xs tabular-nums text-muted-foreground"
              data-testid={`agent-message-duration-${messageId}`}
              title="Time from prompt to this reply"
            >
              <Clock3 className="h-3 w-3" />
              {durationText}
            </span>
          ) : null}

          {showTokensChip && tokensText ? (
            <span
              className="inline-flex items-center gap-1 rounded-full border border-border/60 bg-background/60 px-2 py-0.5 text-2xs tabular-nums text-muted-foreground"
              data-testid={`agent-message-tokens-${messageId}`}
              title={
                meta.metric
                  ? [
                      meta.metric.turnInputTokens
                        ? `in ${meta.metric.turnInputTokens}`
                        : null,
                      meta.metric.turnOutputTokens
                        ? `out ${meta.metric.turnOutputTokens}`
                        : null,
                      meta.metric.model ? meta.metric.model : null,
                      meta.metric.matchKind
                        ? `match ${meta.metric.matchKind}`
                        : null,
                    ]
                      .filter(Boolean)
                      .join(" · ")
                  : meta.metricLoading
                    ? "Loading tokens…"
                    : "Tokens unavailable for this turn"
              }
            >
              <Coins className="h-3 w-3" />
              {tokensText}
              {meta.tokensLabel ? " tokens" : ""}
            </span>
          ) : null}

          {showContextChip ? (
            <button
              aria-label="Context"
              aria-pressed={contextOpen}
              className={cn(
                "inline-flex items-center gap-1 rounded-full border border-border/70 bg-muted/40 px-2 py-0.5 text-2xs font-medium text-muted-foreground transition-colors hover:bg-muted/70 hover:text-foreground",
                contextOpen && "bg-muted/70 text-foreground",
              )}
              data-testid={`agent-message-prompt-context-${messageId}`}
              onClick={() => setContextOpen((v) => !v)}
              title="Context"
              type="button"
            >
              <CheckCheck className="h-3 w-3" />
              <span>Context</span>
            </button>
          ) : null}

          {showThoughtChip ? (
            <button
              aria-expanded={expanded}
              aria-label="Thought"
              className={cn(
                "inline-flex items-center gap-1 rounded-full border border-border/70 bg-muted/40 px-2 py-0.5 text-2xs font-medium text-muted-foreground transition-colors hover:bg-muted/70 hover:text-foreground",
                expanded && "bg-muted/70 text-foreground",
              )}
              data-testid={`agent-message-thinking-toggle-${messageId}`}
              onClick={() => setExpanded((v) => !v)}
              title="Thought"
              type="button"
            >
              <Brain className="h-3 w-3" />
              <span>Thought</span>
              <ChevronDown
                className={cn(
                  "h-3 w-3 transition-transform",
                  expanded && "rotate-180",
                )}
              />
            </button>
          ) : null}
        </span>
      ) : null}

      {expanded && showThoughtChip ? <AgentMessageTurnThinkingCard /> : null}

      {showContextChip || contextOpen ? (
        <PromptContextDialog
          onOpenChange={setContextOpen}
          open={contextOpen}
          sections={meta.promptContextSections}
          setup={setup}
        />
      ) : null}
    </div>
  );
}

/** @deprecated Use AgentMessageTurnFooterChrome — kept as alias during rename. */
export const AgentMessageTurnHeaderChips = AgentMessageTurnFooterChrome;

/** Thinking expand card — sits under the chip row when open. */
export function AgentMessageTurnThinkingCard() {
  const ctx = useChromeContext();
  if (!ctx?.showThoughtChip) return null;

  const { meta, messageId } = ctx;

  const emptyMessage = meta.thinkingLoading
    ? "Loading thinking…"
    : meta.hasThinkingContent
      ? null
      : "No thinking recorded for this turn.";

  return (
    <div
      className="mt-1.5 max-h-64 overflow-y-auto rounded-lg border border-border/70 bg-muted/20 px-2.5 py-2"
      data-testid={`agent-message-thinking-card-${messageId}`}
    >
      {emptyMessage ? (
        <p className="text-xs text-muted-foreground/80">{emptyMessage}</p>
      ) : null}
      <ul className="space-y-2">
        {meta.turnItems.map((item) => {
          if (item.type === "thought") {
            const thoughtText = item.text.trim();
            // Skip empty carriers left by pre-fix coalescer writeback gaps;
            // tools in the same expand still render.
            if (!thoughtText) return null;
            return (
              <li
                className="text-xs leading-5 text-muted-foreground"
                key={item.id}
              >
                <p className="mb-0.5 text-2xs font-medium uppercase tracking-wide text-muted-foreground/80">
                  {item.title || "Thinking"}
                </p>
                <Markdown className="leading-5" content={thoughtText} />
              </li>
            );
          }
          if (item.type === "tool") {
            return (
              <li
                className="flex items-start gap-1.5 text-xs text-muted-foreground"
                key={item.id}
              >
                <span className="mt-0.5 shrink-0 rounded bg-muted px-1 py-0.5 text-2xs font-medium text-foreground/80">
                  tool
                </span>
                <span className="min-w-0 break-words">
                  {item.title}
                  {item.status ? (
                    <span className="text-muted-foreground/70">
                      {" "}
                      · {item.status}
                    </span>
                  ) : null}
                </span>
              </li>
            );
          }
          return null;
        })}
      </ul>
    </div>
  );
}
