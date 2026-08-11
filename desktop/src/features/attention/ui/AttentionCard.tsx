import {
  ArrowUpRight,
  Check,
  ChevronDown,
  ChevronUp,
  Hourglass,
  MoreHorizontal,
  RotateCcw,
} from "lucide-react";
import * as React from "react";

import type { AttentionItem } from "@/features/attention/lib/attention";
import {
  offersHandledElsewhere,
  primaryGenericAction,
  waitingDays,
} from "@/features/attention/lib/attention";
import {
  defaultDeclaredOptions,
  deriveQuickOptions,
} from "@/features/attention/lib/quickOptions";
import { extractTaskLine } from "@/features/attention/lib/taskExtraction";
import type { AskType } from "@/features/attention/lib/taskExtraction";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Markdown } from "@/shared/ui/markdown";

export type AttentionCardAction =
  | "done"
  | "noted"
  | "waiting"
  | "handledElsewhere";

type AttentionCardProps = {
  expanded: boolean;
  isPending: boolean;
  item: AttentionItem;
  onAction: (item: AttentionItem, action: AttentionCardAction) => void;
  onOpen: (item: AttentionItem) => void;
  onOverrideBadge: (id: string, type: AskType) => void;
  onReply: (item: AttentionItem, text: string) => void;
  onRestore: (id: string) => void;
  onToggleExpanded: (id: string) => void;
  selected: boolean;
};

const BADGE_ORDER: AskType[] = [
  "decision",
  "approval",
  "question",
  "review",
  "blocked",
  "headsUp",
];

const BADGES: Record<AskType, { label: string; className: string }> = {
  decision: {
    label: "Decision",
    className: "bg-amber-500/15 text-amber-600 dark:text-amber-400",
  },
  approval: { label: "Approval", className: "bg-primary/10 text-primary" },
  question: {
    label: "Question",
    className: "bg-sky-500/15 text-sky-600 dark:text-sky-400",
  },
  review: {
    label: "Review",
    className: "bg-violet-500/15 text-violet-600 dark:text-violet-400",
  },
  blocked: {
    label: "Blocked",
    className: "bg-red-500/15 text-red-600 dark:text-red-400",
  },
  headsUp: { label: "Heads up", className: "bg-muted text-muted-foreground" },
};

const actionRowClassName =
  "ml-auto flex shrink-0 items-center gap-1 opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100";

function cardHeadline(item: AttentionItem): string {
  // Multi-ask safety rule: never present one ask while hiding another —
  // the badge still reflects the first ask's type.
  const declaredCount = item.declaredAsks?.length ?? 0;
  if (declaredCount > 1) {
    return `${declaredCount} asks`;
  }
  if (item.askCount > 1) {
    return `${item.askCount} questions`;
  }
  if (item.ask) {
    return item.ask;
  }
  const extracted = extractTaskLine(item.inboxItem.item.content);
  return extracted || item.inboxItem.preview || item.inboxItem.subject;
}

/**
 * One attention item: a compact two-line card (badge + bold ask, then
 * sender/channel/staleness meta with hover-revealed actions) that expands
 * on click into the full message with quick-select options and a reply box.
 */
export function AttentionCard({
  expanded,
  isPending,
  item,
  onAction,
  onOpen,
  onOverrideBadge,
  onReply,
  onRestore,
  onToggleExpanded,
  selected,
}: AttentionCardProps) {
  const inboxItem = item.inboxItem;
  const content = inboxItem.item.content;
  const canOpen = Boolean(inboxItem.item.channelId);
  const canPost = canOpen && !isPending;
  const isHeadsUp = item.askType === "headsUp";
  const badge = BADGES[item.askType];
  const declaredAsks = item.declaredAsks ?? [];
  const isMultiAsk = declaredAsks.length > 1;
  const days = waitingDays(
    inboxItem.latestActivityAt,
    Math.floor(Date.now() / 1_000),
  );

  const [replyText, setReplyText] = React.useState("");
  const [badgeMenuOpen, setBadgeMenuOpen] = React.useState(false);
  const [overflowOpen, setOverflowOpen] = React.useState(false);
  // Per-sub-ask local answers, keyed by declared-ask index.
  const [subAnswers, setSubAnswers] = React.useState<Record<number, string>>(
    {},
  );

  const quickOptions = React.useMemo(() => {
    // Declared options take precedence over derived ones; multi-ask cards
    // answer through their sub-items instead.
    if (isMultiAsk) {
      return [];
    }
    if (declaredAsks.length === 1 && declaredAsks[0].options.length > 0) {
      return declaredAsks[0].options;
    }
    return deriveQuickOptions(item.askType, item.ask, content);
  }, [isMultiAsk, declaredAsks, item.askType, item.ask, content]);

  // Locked action matrix: Done only on Blocked, Noted only on Heads up,
  // Handled elsewhere in the overflow everywhere else actionable.
  const genericPrimary = primaryGenericAction(item.askType);
  const hasOverflow = offersHandledElsewhere(item.askType);

  const answeredCount = declaredAsks.filter(
    (_ask, index) => (subAnswers[index] ?? "").trim().length > 0,
  ).length;

  const handleSendAll = () => {
    const lines = declaredAsks
      .map((_ask, index) => {
        const answer = (subAnswers[index] ?? "").trim();
        return answer ? `${index + 1}. ${answer}` : null;
      })
      .filter((line): line is string => line !== null);
    if (lines.length === 0) {
      return;
    }
    onReply(item, lines.join("\n"));
    setSubAnswers({});
  };

  const handleBodyClick = (event: React.MouseEvent<HTMLElement>) => {
    const target = event.target as HTMLElement;
    if (target.closest("button, textarea, a, input")) {
      return;
    }
    onToggleExpanded(item.id);
  };

  const handleReply = () => {
    const text = replyText.trim();
    if (!text) {
      return;
    }
    onReply(item, text);
    setReplyText("");
  };

  return (
    <article
      className={cn(
        "group rounded-xl border border-border/60 bg-background/80 px-4 py-2.5 transition-colors hover:border-border",
        selected && "ring-2 ring-primary/40",
      )}
      data-selected={selected || undefined}
      data-testid={`attention-card-${item.id}`}
    >
      {/* biome-ignore lint/a11y/noStaticElementInteractions: card body toggle is a convenience; the expand button and actions are keyboard-reachable */}
      {/* biome-ignore lint/a11y/useKeyWithClickEvents: keyboard expand is handled by the view-level j/k/e bindings */}
      <div className="cursor-pointer" onClick={handleBodyClick}>
        <div className="flex min-w-0 items-center gap-2">
          <div className="relative shrink-0">
            <button
              aria-expanded={badgeMenuOpen}
              aria-label={`Change type (currently ${badge.label})`}
              className={cn(
                "rounded-full px-2 py-0.5 text-2xs font-medium",
                badge.className,
              )}
              data-testid="attention-badge"
              onClick={() => setBadgeMenuOpen((open) => !open)}
              type="button"
            >
              {badge.label}
            </button>
            {badgeMenuOpen ? (
              <div className="absolute left-0 top-full z-20 mt-1 w-28 rounded-lg border border-border bg-background p-1 shadow-md">
                {BADGE_ORDER.map((type) => (
                  <button
                    className={cn(
                      "block w-full rounded-md px-2 py-1 text-left text-2xs transition-colors hover:bg-muted",
                      type === item.askType
                        ? "font-semibold text-foreground"
                        : "text-muted-foreground",
                    )}
                    data-testid={`attention-badge-option-${type}`}
                    key={type}
                    onClick={() => {
                      onOverrideBadge(item.id, type);
                      setBadgeMenuOpen(false);
                    }}
                    type="button"
                  >
                    {BADGES[type].label}
                  </button>
                ))}
              </div>
            ) : null}
          </div>
          <span
            className="min-w-0 flex-1 truncate text-sm font-semibold text-foreground"
            data-testid="attention-card-ask"
          >
            {cardHeadline(item)}
          </span>
          <span className="ml-auto shrink-0 text-2xs text-muted-foreground">
            {inboxItem.timestampLabel}
          </span>
        </div>
        <div className="mt-1 flex min-w-0 items-center gap-2">
          <span className="min-w-0 truncate text-2xs text-muted-foreground">
            {inboxItem.senderLabel}
            {inboxItem.channelLabel ? ` in #${inboxItem.channelLabel}` : ""}
            {days > 0
              ? ` · waiting ${days} ${days === 1 ? "day" : "days"}`
              : inboxItem.timestampLabel
                ? ` · ${inboxItem.timestampLabel}`
                : ""}
          </span>
          {item.reactivated ? (
            <span className="shrink-0 rounded-full bg-amber-500/15 px-2 py-0.5 text-2xs font-medium text-amber-600 dark:text-amber-400">
              New activity
            </span>
          ) : null}
          {item.responded ? (
            <span
              className="shrink-0 rounded-full bg-emerald-500/15 px-2 py-0.5 text-2xs font-medium text-emerald-600 dark:text-emerald-400"
              data-testid="attention-responded-badge"
            >
              You replied
            </span>
          ) : null}
          <div className={actionRowClassName}>
            {item.zone === "needsMe" && !isHeadsUp ? (
              <>
                <Button
                  data-testid="attention-action-waiting"
                  disabled={!canPost}
                  onClick={() => onAction(item, "waiting")}
                  size="xs"
                  type="button"
                  variant="ghost"
                >
                  <Hourglass />
                  Waiting
                </Button>
                {genericPrimary === "done" ? (
                  <Button
                    data-testid="attention-action-done"
                    disabled={!canPost}
                    onClick={() => onAction(item, "done")}
                    size="xs"
                    type="button"
                    variant="ghost"
                  >
                    <Check />
                    Done
                  </Button>
                ) : null}
                {hasOverflow ? (
                  <div className="relative">
                    <Button
                      aria-expanded={overflowOpen}
                      aria-label="More actions"
                      data-testid="attention-action-overflow"
                      onClick={() => setOverflowOpen((open) => !open)}
                      size="xs"
                      type="button"
                      variant="ghost"
                    >
                      <MoreHorizontal />
                    </Button>
                    {overflowOpen ? (
                      <div className="absolute right-0 top-full z-20 mt-1 w-40 rounded-lg border border-border bg-background p-1 shadow-md">
                        <button
                          className="block w-full rounded-md px-2 py-1 text-left text-2xs text-muted-foreground transition-colors hover:bg-muted"
                          data-testid="attention-action-handled-elsewhere"
                          disabled={!canPost}
                          onClick={() => {
                            setOverflowOpen(false);
                            onAction(item, "handledElsewhere");
                          }}
                          type="button"
                        >
                          Handled elsewhere
                        </button>
                      </div>
                    ) : null}
                  </div>
                ) : null}
              </>
            ) : null}
            {item.zone === "needsMe" && isHeadsUp ? (
              <Button
                data-testid="attention-action-noted"
                // Noted on a To note item is local-only — it needs no
                // channel to post into, only a non-pending card.
                disabled={isPending}
                onClick={() => onAction(item, "noted")}
                size="xs"
                type="button"
                variant="ghost"
              >
                <Check />
                Noted
              </Button>
            ) : null}
            {item.zone === "waiting" ? (
              <>
                <Button
                  data-testid="attention-action-done"
                  disabled={!canPost}
                  onClick={() => onAction(item, "done")}
                  size="xs"
                  type="button"
                  variant="ghost"
                >
                  <Check />
                  Done
                </Button>
                <Button
                  data-testid="attention-action-restore"
                  disabled={isPending}
                  onClick={() => onRestore(item.id)}
                  size="xs"
                  type="button"
                  variant="ghost"
                >
                  <RotateCcw />
                  Needs me
                </Button>
              </>
            ) : null}
            {item.zone === "done" ? (
              <Button
                data-testid="attention-action-restore"
                disabled={isPending}
                onClick={() => onRestore(item.id)}
                size="xs"
                type="button"
                variant="ghost"
              >
                <RotateCcw />
                Restore
              </Button>
            ) : null}
            <Button
              data-testid="attention-action-open"
              disabled={!canOpen}
              onClick={() => onOpen(item)}
              size="xs"
              type="button"
              variant="outline"
            >
              <ArrowUpRight />
              Open
            </Button>
            {item.zone === "needsMe" && !isHeadsUp ? (
              <Button
                aria-label={expanded ? "Collapse" : "Expand"}
                data-testid="attention-action-expand"
                onClick={() => onToggleExpanded(item.id)}
                size="xs"
                type="button"
                variant="ghost"
              >
                {expanded ? <ChevronUp /> : <ChevronDown />}
              </Button>
            ) : null}
          </div>
        </div>
        {/* Options are the point of the card: visible collapsed, one click
            clears the row without an expand. */}
        {item.zone === "needsMe" && !isHeadsUp && !expanded ? (
          isMultiAsk ? (
            <div
              className="mt-1.5 flex flex-col gap-0.5"
              data-testid="attention-collapsed-subasks"
            >
              {declaredAsks.map((declaredAsk, index) => (
                <p
                  className="truncate text-2xs text-muted-foreground"
                  key={`${declaredAsk.type}-${declaredAsk.ask}`}
                >
                  {index + 1}. {declaredAsk.ask}
                </p>
              ))}
            </div>
          ) : quickOptions.length > 0 ? (
            <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
              {quickOptions.map((option) => (
                <Button
                  data-testid="attention-quick-option"
                  disabled={!canPost}
                  key={option}
                  onClick={() => onReply(item, option)}
                  size="xs"
                  type="button"
                  variant="default"
                >
                  {option}
                </Button>
              ))}
            </div>
          ) : null
        ) : null}
      </div>
      {expanded ? (
        <div
          className="mt-3 border-t border-border/60 pt-3"
          data-testid="attention-card-expanded"
        >
          {/* Reading order: ask, options, body, reply — the options never
              sit below the fold of a long message. */}
          {isMultiAsk ? (
            <div className="flex flex-col gap-2">
              <p
                className="text-2xs text-muted-foreground"
                data-testid="attention-subitems-progress"
              >
                {answeredCount} of {declaredAsks.length} answered
              </p>
              {declaredAsks.map((declaredAsk, index) => {
                // Optionless declared asks fall back to the closed
                // per-type defaults (fixture 4: a bare Review sub-ask
                // still gets its one-click pair plus the way out).
                const subOptions =
                  declaredAsk.options.length > 0
                    ? declaredAsk.options
                    : defaultDeclaredOptions(declaredAsk.type);
                return (
                  <div
                    className="rounded-lg border border-border/60 p-2.5"
                    data-testid="attention-subitem"
                    key={`${declaredAsk.type}-${declaredAsk.ask}`}
                  >
                    <p className="text-sm font-medium text-foreground">
                      {index + 1}. {declaredAsk.ask}
                    </p>
                    {subOptions.length > 0 ? (
                      <div className="mt-2 flex flex-wrap items-center gap-1.5">
                        {subOptions.map((option) => (
                          <Button
                            data-testid="attention-subitem-option"
                            disabled={!canPost}
                            key={option}
                            onClick={() =>
                              setSubAnswers((prev) => ({
                                ...prev,
                                [index]: option,
                              }))
                            }
                            size="xs"
                            type="button"
                            variant={
                              subAnswers[index] === option
                                ? "default"
                                : "outline"
                            }
                          >
                            {option}
                          </Button>
                        ))}
                      </div>
                    ) : null}
                    <input
                      className="mt-2 w-full rounded-md border border-border/60 bg-background px-2 py-1 text-sm text-foreground outline-none placeholder:text-muted-foreground focus:border-primary/50"
                      data-testid="attention-subitem-input"
                      onChange={(event) =>
                        setSubAnswers((prev) => ({
                          ...prev,
                          [index]: event.target.value,
                        }))
                      }
                      placeholder="Type an answer…"
                      type="text"
                      value={subAnswers[index] ?? ""}
                    />
                  </div>
                );
              })}
              <div className="flex items-center justify-end gap-1">
                <Button
                  data-testid="attention-action-reply"
                  disabled={!canPost || answeredCount === 0}
                  onClick={handleSendAll}
                  size="xs"
                  type="button"
                >
                  Send all
                </Button>
              </div>
            </div>
          ) : null}
          {!isMultiAsk && quickOptions.length > 0 ? (
            <div className="flex flex-wrap items-center gap-1.5">
              {quickOptions.map((option) => (
                <Button
                  data-testid="attention-quick-option"
                  disabled={!canPost}
                  key={option}
                  onClick={() => onReply(item, option)}
                  size="xs"
                  type="button"
                  variant="default"
                >
                  {option}
                </Button>
              ))}
            </div>
          ) : null}
          <div className="mt-3 text-sm text-foreground/90">
            <Markdown content={content} />
          </div>
          {isMultiAsk ? null : (
            <div className="mt-3 flex items-end gap-2">
              <textarea
                // biome-ignore lint/a11y/noAutofocus: expanding a card is an explicit intent to reply
                autoFocus
                className="min-h-9 w-full flex-1 resize-y rounded-lg border border-border/60 bg-background px-3 py-2 text-sm text-foreground outline-none placeholder:text-muted-foreground focus:min-h-16 focus:border-primary/50"
                data-testid="attention-reply-input"
                onChange={(event) => setReplyText(event.target.value)}
                placeholder={
                  canOpen
                    ? "Reply in the source thread…"
                    : "This item has no channel to reply into."
                }
                value={replyText}
              />
              <Button
                data-testid="attention-action-reply"
                disabled={!canPost || replyText.trim().length === 0}
                onClick={handleReply}
                size="xs"
                type="button"
              >
                Reply
              </Button>
            </div>
          )}
        </div>
      ) : null}
    </article>
  );
}
