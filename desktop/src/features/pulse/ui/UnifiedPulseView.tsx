import { ArrowUp, Inbox, Search } from "lucide-react";
import * as React from "react";
import { TerminalSurfaceContext } from "@/features/terminal/TerminalSurfaceContext";
import { useTerminalPanel } from "@/features/terminal/terminalPanelStore";
import { cn } from "@/shared/lib/cn";
import { useUnifiedPulseFeed } from "@/features/pulse/useUnifiedPulseFeed";
import {
  matchesPulseFilter,
  type PulseConversation,
} from "@/features/pulse/lib/unifiedFeed";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Skeleton } from "@/shared/ui/skeleton";
import { VirtualizedList } from "@/shared/ui/VirtualizedList";
import { ConversationCard } from "./ConversationCard";
import {
  isPulseWorkspacePage,
  type PulseView,
  PULSE_WORKSPACE_KEYS,
  CLEAR_WORKSPACE_PANELS,
} from "../lib/workspaceNavigation";
import { PulseAppNavigation, type PulseApp } from "./PulseAppNavigation";
import { PulseWorkspacePage } from "./PulseWorkspacePage";
import { PulseCombinedView } from "./PulseCombinedView";
import { PulseWindowActions } from "./PulseWindowActions";
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
  ...PULSE_CONVERSATION_KEYS,
  ...PULSE_WORKSPACE_KEYS,
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
  const terminal = React.useContext(TerminalSurfaceContext);
  const terminalPanel = useTerminalPanel();
  const expanded =
    terminalPanel.mode !== "closed" ||
    Boolean(
      values.channelManagement ||
        values.profile ||
        values.profilePersona ||
        values.agentSession,
    );
  const filter: PulseView = isPulseWorkspacePage(values.feed)
    ? values.feed
    : values.feed === "search"
      ? "search"
      : values.feed && values.feed !== "all"
        ? "conversation"
        : "all";
  React.useEffect(() => {
    if (values.feed === "dm" || values.feed === "channel") {
      applyPatch(
        {
          feed: "conversation",
          conversation: values.feed === "dm" ? values.dm : values.channel,
          dm: null,
          channel: null,
        },
        { replace: true },
      );
    }
  }, [values.feed, values.dm, values.channel, applyPatch]);
  const setFilter = (next: PulseView) => {
    if (!allowNavigation({ kind: "route", href: `/pulse?feed=${next}` }))
      return false;
    applyPatch({
      ...CLEAR_CONVERSATION_PANELS,
      ...CLEAR_WORKSPACE_PANELS,
      feed: next === "all" ? null : next,
    });
    setBriefingFilter(null);
    return true;
  };
  const activeApp: PulseApp = isPulseWorkspacePage(filter)
    ? filter
    : "messages";
  const lastMessageView = React.useRef<PulseView>(
    values.conversation ? "conversation" : "all",
  );
  React.useEffect(() => {
    if (!isPulseWorkspacePage(filter)) lastMessageView.current = filter;
  }, [filter]);
  const selectApp = (app: PulseApp) => {
    if (app === activeApp) return;
    setFilter(app === "messages" ? lastMessageView.current : app);
  };
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
  React.useEffect(() => {
    if (newCount > 0 && (!scrollElement || scrollElement.scrollTop <= 24)) {
      setAccepted({
        scope: feed.scope,
        ids: feed.conversations.map((item) => item.id),
      });
    }
  }, [feed.conversations, feed.scope, newCount, scrollElement]);
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
        "all",
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
              divider={filter === "conversation"}
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
        Recent activity · Updates live while focused
        <br />
        Private conversations stay private. Replies go to their original thread.
      </footer>
    </>
  );
  const content = (
    <>
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
          <p>Some activity couldn’t be loaded. Your feed may be out of date.</p>
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
      {filter !== "all" ? feedContent : null}
    </>
  );
  return (
    <div
      className="flex min-h-0 flex-1 gap-2 px-[8px] py-[12px]"
      data-testid="unified-pulse"
    >
      <PulseAppNavigation active={activeApp} onSelect={selectApp} />
      <div className="flex min-h-0 min-w-0 flex-1">
        <div
          data-testid="pulse-main-container"
          className={cn(
            "relative mx-auto flex min-h-0 w-full flex-1 flex-col overflow-hidden rounded-[24px] bg-background",
            !expanded && "max-w-[960px]",
          )}
          data-expanded={expanded}
        >
          <PulseWindowActions
            onRefresh={() => void feed.refresh()}
            refreshing={feed.query.isFetching}
          />
          <div className="pulse-conversation-workspace flex min-h-0 flex-1">
            <div
              className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden"
              data-testid="pulse-scroll-area"
            >
              {isPulseWorkspacePage(filter) ? (
                <PulseWorkspacePage page={filter} />
              ) : (
                <PulseCombinedView
                  channels={feed.channels}
                  conversations={feed.conversations}
                  currentPubkey={currentPubkey}
                  scrollRef={setScrollElement}
                  view={filter}
                  onSelectView={setFilter}
                >
                  {content}
                </PulseCombinedView>
              )}
            </div>
            <div
              className="pulse-terminal-side-host"
              data-testid="pulse-terminal-panel"
            >
              {terminal}
            </div>
          </div>
        </div>
      </div>
      {!expanded && (
        <div aria-hidden="true" className="w-[64px] shrink-0 lg:w-[180px]" />
      )}
    </div>
  );
}
