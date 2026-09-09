import { ArrowUp, Bot, Hash, Inbox, MessageCircle, Search } from "lucide-react";
import * as React from "react";
import { useUnifiedPulseFeed } from "@/features/pulse/useUnifiedPulseFeed";
import {
  matchesPulseFilter,
  type FeedFilter,
  type PulseConversation,
} from "@/features/pulse/lib/unifiedFeed";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Skeleton } from "@/shared/ui/skeleton";
import { VirtualizedList } from "@/shared/ui/VirtualizedList";
import { ConversationCard } from "./ConversationCard";
import { PulseCombinedView } from "./PulseCombinedView";
import { PulseVariationMenu } from "./PulseVariationMenu";
import { PulseDmView } from "./PulseDmView";
import { PulseChannelsView } from "./PulseChannelsView";
import {
  CLEAR_CONVERSATION_PANELS,
  PULSE_CONVERSATION_KEYS,
} from "../lib/pulsePanelState";
import { useHistorySearchState } from "@/shared/hooks/useHistorySearchState";
import { allowNavigation } from "@/app/navigation/navigationGuard";
import { useAppShell } from "@/app/AppShellContext";
import type { BriefingKind } from "../lib/pulseBriefing";
import { PulseBriefing } from "./PulseBriefing";
import { buildSummaryInput } from "../lib/pulseSummary";
import { usePulseSummary } from "../usePulseSummary";
const FEED_SEARCH_KEYS = [
  "feed",
  "layout",
  ...PULSE_CONVERSATION_KEYS,
] as const;

type PulseView = FeedFilter | "search" | "conversation";

const separateFilters = [
  { id: "search", label: "Search", icon: Search },
  { id: "all", label: "For you", icon: Inbox },
  { id: "dm", label: "DMs", icon: MessageCircle },
  { id: "channel", label: "Channels", icon: Hash },
  { id: "agent", label: "Agents", icon: Bot },
] as const;

const combinedFilters = [
  { id: "search", label: "Search", icon: Search },
  { id: "all", label: "For you", icon: Inbox },
  { id: "conversation", label: "Conversations", icon: MessageCircle },
  { id: "agent", label: "Agents", icon: Bot },
] as const;

export function UnifiedPulseView({
  currentPubkey,
}: {
  currentPubkey?: string;
}) {
  const feed = useUnifiedPulseFeed(currentPubkey);
  const reads = useAppShell();
  const [briefingFilter, setBriefingFilter] =
    React.useState<BriefingKind | null>(null);
  const { values, applyPatch } = useHistorySearchState(FEED_SEARCH_KEYS);
  const combined = values.layout === "combined";
  const filters = combined ? combinedFilters : separateFilters;
  const filter: PulseView = filters.some((f) => f.id === values.feed)
    ? (values.feed as PulseView)
    : "all";
  const setFilter = (next: PulseView) => {
    if (!allowNavigation({ kind: "route", href: `/pulse?feed=${next}` }))
      return false;
    applyPatch({
      ...CLEAR_CONVERSATION_PANELS,
      feed: next === "all" ? null : next,
    });
    setBriefingFilter(null);
    return true;
  };
  const setVariation = (next: "separate" | "combined") => {
    if ((next === "combined") === combined) return;
    if (!allowNavigation({ kind: "route", href: `/pulse?layout=${next}` }))
      return;
    applyPatch({
      ...CLEAR_CONVERSATION_PANELS,
      layout: next === "combined" ? "combined" : null,
      feed: next === "combined" ? "conversation" : null,
    });
    setBriefingFilter(null);
  };
  const isSplitView =
    filter === "dm" || filter === "channel" || filter === "conversation";
  const [search, setSearch] = React.useState("");
  const [scrollElement, setScrollElement] =
    React.useState<HTMLDivElement | null>(null);
  // Split views replace the scroll host. Notify the virtualizer after the new
  // host attaches, including when cached feed content mounts in the same commit.
  const scrollRef = React.useMemo(
    () => ({ current: scrollElement }),
    [scrollElement],
  );
  // Freeze order, not content: edits/deletions update, new conversations wait.
  const [accepted, setAccepted] = React.useState<{
    scope: string;
    ids: string[];
  } | null>(null);
  const ids = feed.conversations.map((item) => item.id);
  const acceptedIds = accepted?.scope === feed.scope ? accepted.ids : ids;
  React.useEffect(() => {
    if (feed.query.isSuccess && accepted?.scope !== feed.scope)
      setAccepted({
        scope: feed.scope,
        ids: feed.conversations.map((item) => item.id),
      });
  }, [accepted?.scope, feed.scope, feed.query.isSuccess, feed.conversations]);
  const acceptedSet = new Set(acceptedIds);
  const newCount = ids.filter((id) => !acceptedSet.has(id)).length;
  const byId = new Map(feed.conversations.map((item) => [item.id, item]));
  const acceptedConversations = acceptedIds
    .map((id) => byId.get(id))
    .filter((item): item is PulseConversation => Boolean(item));
  const summaryInput = buildSummaryInput(
    feed.conversations,
    currentPubkey ?? "",
    reads,
    `${currentPubkey}:${feed.scope}`,
    Date.now() / 1000,
  );
  const summary = usePulseSummary(
    summaryInput,
    filter === "all" && !feed.isLoading && Boolean(currentPubkey),
  );
  const briefing = (summary.data ?? []).filter((group) =>
    [...group.ids].every((id) => byId.has(id)),
  );
  const focusedIds = briefing.find(
    (group) => group.kind === briefingFilter,
  )?.ids;
  const visible = acceptedConversations
    .filter((item) => filter !== "conversation" || Boolean(item.channel))
    .filter((item) =>
      matchesPulseFilter(
        item,
        filter === "search" || filter === "conversation" ? "all" : filter,
        false,
        false,
        filter === "search" ? search : "",
      ),
    )
    .filter(
      (item) =>
        filter !== "search" || !briefingFilter || focusedIds?.has(item.id),
    );
  const showLatest = () => {
    setAccepted({ scope: feed.scope, ids });
    scrollRef.current?.scrollTo({ top: 0 });
  };
  const refresh = async () => {
    const result = await feed.query.refetch();
    if (result.isSuccess) setAccepted(null);
  };
  const feedContent = (
    <>
      {newCount > 0 && (
        <div className="sticky top-16 z-10 flex justify-center py-3">
          <Button
            className="gap-2 rounded-full shadow-lg"
            size="sm"
            onClick={showLatest}
          >
            <ArrowUp aria-hidden className="h-3.5 w-3.5" />
            {newCount} new conversations
          </Button>
        </div>
      )}
      {feed.isLoading ? (
        <div role="status" aria-label="Loading feed" className="space-y-8 p-7">
          {[0, 1, 2].map((id) => (
            <div key={id} className="flex gap-3">
              <Skeleton className="h-9 w-9 rounded-full" />
              <div className="flex-1 space-y-3">
                <Skeleton className="h-3 w-32" />
                <Skeleton className="h-16 w-full" />
              </div>
            </div>
          ))}
        </div>
      ) : visible.length ? (
        <VirtualizedList
          items={visible}
          getItemKey={(item) => item.id}
          estimateSize={260}
          scrollRef={scrollRef}
          renderItem={(item) => (
            <ConversationCard
              item={item}
              currentPubkey={currentPubkey}
              profiles={feed.profiles}
              onRefresh={() => void refresh()}
            />
          )}
        />
      ) : (
        !feed.error && (
          <div className="px-6 py-20 text-center">
            <Inbox
              aria-hidden
              className="mx-auto mb-4 h-7 w-7 text-muted-foreground/60"
            />
            <h2 className="text-base font-medium">A little quiet here</h2>
            <p className="mx-auto mt-2 max-w-xs text-sm text-muted-foreground">
              {search || filter !== "all" || briefingFilter
                ? "No conversations match these filters. Try another view or search term."
                : "Messages from your selected channels, DMs, and people you follow will appear here."}
            </p>
            {(search || filter !== "all" || briefingFilter) && (
              <Button
                variant="outline"
                size="sm"
                className="mt-5 rounded-full"
                onClick={() => {
                  setFilter("search");
                  setSearch("");
                }}
              >
                Clear filters
              </Button>
            )}
          </div>
        )
      )}
      <footer className="px-6 py-7 text-center text-2xs text-muted-foreground">
        Recent activity · Refreshes every 30 seconds while focused
        <br />
        Private conversations stay private. Replies go to their original thread.
      </footer>
    </>
  );
  return (
    <div
      className="flex min-h-0 flex-1 flex-col bg-[#f7f7f8] py-[12px] dark:bg-muted/20"
      data-testid="unified-pulse"
    >
      <div
        data-testid="pulse-main-container"
        className="relative mx-auto flex min-h-0 w-full max-w-[800px] flex-1 flex-col overflow-hidden rounded-[24px] border border-border/40 bg-background"
      >
        <div className="z-20 flex shrink-0 items-center border-b border-border/50 bg-background px-3 sm:px-5">
          <div className="min-w-0 flex-1 overflow-x-auto">
            <fieldset
              aria-label="Pulse views"
              data-testid="pulse-tabs"
              className="flex min-w-max"
            >
              {filters.map(({ id, label, icon: Icon }) => (
                <button
                  key={id}
                  type="button"
                  aria-pressed={filter === id}
                  onClick={() => setFilter(id)}
                  className={`relative flex flex-1 items-center justify-center gap-2 whitespace-nowrap px-3 py-4 text-xs font-medium transition-colors focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring ${filter === id ? "text-foreground" : "text-muted-foreground hover:bg-muted/40 hover:text-foreground"}`}
                >
                  <Icon aria-hidden className="h-4 w-4" />
                  {label}
                  {filter === id && (
                    <span
                      aria-hidden
                      className="absolute inset-x-3 bottom-0 h-0.5 rounded-full bg-primary"
                    />
                  )}
                </button>
              ))}
            </fieldset>
          </div>
          <PulseVariationMenu
            value={combined ? "combined" : "separate"}
            onChange={setVariation}
          />
        </div>
        <div
          className={`min-h-0 flex-1 ${isSplitView ? "flex flex-col overflow-hidden" : "overflow-y-auto"}`}
          ref={isSplitView ? undefined : setScrollElement}
          data-testid="pulse-scroll-area"
        >
          {filter === "search" && (
            <div className="px-5 py-5 sm:px-7">
              <div className="relative w-full">
                <Search
                  aria-hidden
                  className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground"
                />
                <Input
                  type="search"
                  aria-label="Search loaded feed"
                  placeholder="Search this feed"
                  value={search}
                  onChange={(e) => setSearch(e.target.value)}
                  className="h-11 w-full rounded-xl border-0 bg-muted/30 pl-10 text-sm shadow-none"
                />
              </div>
              {briefingFilter && (
                <Button
                  variant="ghost"
                  size="sm"
                  className="mt-2"
                  onClick={() => setBriefingFilter(null)}
                >
                  Show all activity
                </Button>
              )}
            </div>
          )}
          {filter === "all" && (
            <PulseBriefing
              groups={briefing}
              conversations={byId}
              profiles={feed.profiles}
              currentPubkey={currentPubkey}
              onRefresh={() => void refresh()}
              onSelect={(kind) => {
                if (!setFilter("search")) return;
                setBriefingFilter(kind);
                setAccepted({ scope: feed.scope, ids });
                setSearch("");
                scrollRef.current?.scrollTo({ top: 0 });
              }}
              loading={
                feed.isLoading ||
                (summaryInput.conversations.length > 0 && summary.isPending)
              }
              hasError={Boolean(feed.error || summary.error)}
              onRetry={() => {
                feed.retry();
                void summary.refetch();
              }}
            />
          )}
          {filter !== "all" && feed.error && (
            <div
              role="alert"
              className="m-5 rounded-xl border border-destructive/30 bg-destructive/5 p-4 text-sm"
            >
              <p>
                Some activity couldn’t be loaded. Your feed may be out of date.
              </p>
              <Button
                size="sm"
                variant="outline"
                className="mt-2"
                onClick={feed.retry}
              >
                Try again
              </Button>
            </div>
          )}
          {filter === "conversation" ? (
            <PulseCombinedView
              channels={feed.channels}
              conversations={feed.conversations}
              currentPubkey={currentPubkey}
              scrollRef={setScrollElement}
            >
              {feedContent}
            </PulseCombinedView>
          ) : filter === "dm" ? (
            <PulseDmView
              channels={feed.channels}
              currentPubkey={currentPubkey}
              search={""}
            />
          ) : filter === "channel" ? (
            <PulseChannelsView
              channels={feed.channels}
              search={""}
              scrollRef={setScrollElement}
            >
              {feedContent}
            </PulseChannelsView>
          ) : filter !== "all" ? (
            feedContent
          ) : null}
        </div>
      </div>
    </div>
  );
}
