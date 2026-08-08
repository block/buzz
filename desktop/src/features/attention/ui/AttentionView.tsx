import * as React from "react";

import type {
  AskType,
  AttentionItem,
  AttentionProjection,
  AttentionZone,
} from "@/features/attention/lib/attention";
import { isSameLocalDay } from "@/features/attention/lib/attention";
import {
  AttentionCard,
  type AttentionCardAction,
} from "@/features/attention/ui/AttentionCard";
import { cn } from "@/shared/lib/cn";
import { TopChromeInsetHeader } from "@/shared/layout/TopChromeInsetHeader";

type AttentionViewProps = {
  errorMessage?: string;
  isLoading: boolean;
  onAction: (item: AttentionItem, action: AttentionCardAction) => void;
  onNoteAll: () => void;
  onOpen: (item: AttentionItem) => void;
  onOverrideBadge: (id: string, type: AskType) => void;
  onReply: (item: AttentionItem, text: string) => void;
  onRestore: (id: string) => void;
  pendingIds: ReadonlySet<string>;
  projection: AttentionProjection;
};

const ZONE_TABS: Array<{ zone: AttentionZone; label: string }> = [
  { zone: "needsMe", label: "Needs Me" },
  { zone: "waiting", label: "Waiting" },
  { zone: "done", label: "Done" },
];

const tabButtonClassName =
  "h-7 rounded-full border border-transparent px-2.5 text-2xs font-medium text-muted-foreground transition-colors hover:text-foreground data-[active=true]:border-border/70 data-[active=true]:bg-background/80 data-[active=true]:text-foreground data-[active=true]:shadow-xs";

const sectionHeaderClassName =
  "px-1 pb-1.5 pt-3 text-2xs font-semibold uppercase tracking-wide text-muted-foreground";

/**
 * The Attention screen: tabbed Needs Me / Waiting / Done views over the
 * attention projection, with j/k keyboard navigation and per-card actions.
 */
export function AttentionView({
  errorMessage,
  isLoading,
  onAction,
  onNoteAll,
  onOpen,
  onOverrideBadge,
  onReply,
  onRestore,
  pendingIds,
  projection,
}: AttentionViewProps) {
  const [activeZone, setActiveZone] = React.useState<AttentionZone>("needsMe");
  const [selectedId, setSelectedId] = React.useState<string | null>(null);
  const [expandedId, setExpandedId] = React.useState<string | null>(null);

  const nowSeconds = Math.floor(Date.now() / 1_000);
  const overdue = projection.needsMe.filter(
    (item) => !isSameLocalDay(item.inboxItem.latestActivityAt, nowSeconds),
  );
  const today = projection.needsMe.filter((item) =>
    isSameLocalDay(item.inboxItem.latestActivityAt, nowSeconds),
  );

  const visibleItems: AttentionItem[] =
    activeZone === "needsMe"
      ? [...overdue, ...today, ...projection.headsUp]
      : activeZone === "waiting"
        ? projection.waiting
        : projection.done;

  const switchZone = (zone: AttentionZone) => {
    setActiveZone(zone);
    setSelectedId(null);
    setExpandedId(null);
  };

  const toggleExpanded = React.useCallback((id: string) => {
    setExpandedId((prev) => (prev === id ? null : id));
    setSelectedId(id);
  }, []);

  const moveSelection = (delta: number) => {
    if (visibleItems.length === 0) {
      return;
    }
    const index = visibleItems.findIndex((item) => item.id === selectedId);
    const next =
      index === -1
        ? delta > 0
          ? 0
          : visibleItems.length - 1
        : Math.min(Math.max(index + delta, 0), visibleItems.length - 1);
    const nextId = visibleItems[next].id;
    setSelectedId(nextId);
    document
      .querySelector(`[data-testid="attention-card-${nextId}"]`)
      ?.scrollIntoView({ block: "nearest" });
  };

  const handleKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    const target = event.target as HTMLElement;
    if (target.closest("textarea, input, [contenteditable='true']")) {
      return;
    }
    const selected =
      visibleItems.find((item) => item.id === selectedId) ?? null;
    switch (event.key) {
      case "j":
        event.preventDefault();
        moveSelection(1);
        break;
      case "k":
        event.preventDefault();
        moveSelection(-1);
        break;
      case "e":
        if (selected) {
          event.preventDefault();
          toggleExpanded(selected.id);
        }
        break;
      case "r":
        if (selected) {
          event.preventDefault();
          setExpandedId(selected.id);
        }
        break;
      case "w":
        if (
          selected &&
          selected.zone === "needsMe" &&
          selected.askType !== "headsUp"
        ) {
          event.preventDefault();
          onAction(selected, "waiting");
        }
        break;
      case "d":
        if (
          selected &&
          (selected.zone === "waiting" ||
            (selected.zone === "needsMe" && selected.askType !== "headsUp"))
        ) {
          event.preventDefault();
          onAction(selected, "done");
        }
        break;
      case "n":
        if (
          selected &&
          selected.zone === "needsMe" &&
          selected.askType === "headsUp"
        ) {
          event.preventDefault();
          onAction(selected, "noted");
        }
        break;
      case "o":
        if (selected) {
          event.preventDefault();
          onOpen(selected);
        }
        break;
      default:
        break;
    }
  };

  const renderCard = (item: AttentionItem) => (
    <AttentionCard
      expanded={expandedId === item.id}
      isPending={pendingIds.has(item.id)}
      item={item}
      key={item.id}
      onAction={onAction}
      onOpen={onOpen}
      onOverrideBadge={onOverrideBadge}
      onReply={onReply}
      onRestore={onRestore}
      onToggleExpanded={toggleExpanded}
      selected={selectedId === item.id}
    />
  );

  const needsMeEmpty =
    projection.needsMe.length === 0 && projection.headsUp.length === 0;

  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: list-level shortcuts; every action is also reachable via the focusable card buttons
    <div
      className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden outline-none"
      data-testid="attention-view"
      onKeyDown={handleKeyDown}
      // biome-ignore lint/a11y/noNoninteractiveTabindex: the view is a keyboard command surface (j/k/e/r/w/d/n/o) like a mail list
      tabIndex={0}
    >
      <TopChromeInsetHeader data-tauri-drag-region flush>
        <header className="min-w-0 cursor-default select-none px-5 py-2">
          <div className="flex h-9 min-w-0 items-center gap-2.5">
            <div className="min-w-0 flex-1">
              <h1 className="truncate text-sm font-semibold text-foreground">
                Attention
              </h1>
              <p className="truncate text-2xs text-muted-foreground">
                What needs you right now, across every channel and agent
              </p>
            </div>
          </div>
        </header>
      </TopChromeInsetHeader>
      <div className="flex items-center gap-1 px-5 py-2">
        {ZONE_TABS.map((tab) => (
          <button
            className={tabButtonClassName}
            data-active={activeZone === tab.zone}
            data-testid={`attention-tab-${tab.zone}`}
            key={tab.zone}
            onClick={() => switchZone(tab.zone)}
            type="button"
          >
            {tab.label}
            {tab.zone === "needsMe" ? (
              <span
                className={cn(
                  "ml-1.5 rounded-full px-1.5 text-3xs",
                  activeZone === tab.zone
                    ? "bg-primary/15 text-primary"
                    : "bg-muted text-muted-foreground",
                )}
              >
                <span data-testid="attention-count-needs">
                  {projection.needsMe.length} need you
                </span>
                <span data-testid="attention-count-note">
                  {" "}
                  · {projection.headsUp.length} to note
                </span>
              </span>
            ) : (
              <span
                className={cn(
                  "ml-1.5 rounded-full px-1.5 text-3xs",
                  activeZone === tab.zone
                    ? "bg-primary/15 text-primary"
                    : "bg-muted text-muted-foreground",
                )}
              >
                {
                  (tab.zone === "waiting"
                    ? projection.waiting
                    : projection.done
                  ).length
                }
              </span>
            )}
          </button>
        ))}
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto px-5 pb-5">
        {errorMessage ? (
          <div className="rounded-xl border border-destructive/40 bg-destructive/5 px-4 py-3 text-sm text-destructive">
            {errorMessage}
          </div>
        ) : isLoading ? (
          <p className="px-1 py-8 text-sm text-muted-foreground">
            Loading your attention items…
          </p>
        ) : activeZone === "needsMe" ? (
          needsMeEmpty ? (
            <div
              className="flex flex-1 flex-col items-center justify-center gap-3 rounded-2xl border border-dashed border-border/60 px-4 py-12 text-center"
              data-testid="attention-empty-state"
            >
              <p className="text-sm text-muted-foreground">
                Nothing needs you. {projection.waiting.length} items are waiting
                on other people.
              </p>
              <button
                className="rounded-full border border-border/70 px-3 py-1 text-2xs font-medium text-foreground transition-colors hover:bg-muted"
                onClick={() => switchZone("waiting")}
                type="button"
              >
                Show Waiting
              </button>
            </div>
          ) : (
            <div className="flex flex-col" data-testid="attention-card-list">
              {overdue.length > 0 ? (
                <section data-testid="attention-section-overdue">
                  <h2 className={sectionHeaderClassName}>
                    Waiting on you since
                  </h2>
                  <div className="flex flex-col gap-2">
                    {overdue.map(renderCard)}
                  </div>
                </section>
              ) : null}
              {today.length > 0 ? (
                <section data-testid="attention-section-today">
                  <h2 className={sectionHeaderClassName}>Today</h2>
                  <div className="flex flex-col gap-2">
                    {today.map(renderCard)}
                  </div>
                </section>
              ) : null}
              {projection.headsUp.length > 0 ? (
                <section data-testid="attention-section-note">
                  <div className="flex items-center justify-between">
                    <h2 className={sectionHeaderClassName}>To note</h2>
                    <button
                      className="rounded-full px-2 py-0.5 text-2xs font-medium text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
                      data-testid="attention-note-all"
                      onClick={onNoteAll}
                      type="button"
                    >
                      Note all
                    </button>
                  </div>
                  <div className="flex flex-col gap-2">
                    {projection.headsUp.map(renderCard)}
                  </div>
                </section>
              ) : null}
            </div>
          )
        ) : visibleItems.length === 0 ? (
          <div
            className="flex flex-1 flex-col items-center justify-center gap-2 rounded-2xl border border-dashed border-border/60 px-4 py-12 text-center"
            data-testid="attention-empty-state"
          >
            <p className="text-sm text-muted-foreground">
              {activeZone === "waiting"
                ? "Nothing parked. Items you are waiting on others for land here."
                : "Items you resolve stay here for 7 days."}
            </p>
          </div>
        ) : (
          <div
            className="flex flex-col gap-2"
            data-testid="attention-card-list"
          >
            {visibleItems.map(renderCard)}
          </div>
        )}
      </div>
    </div>
  );
}
