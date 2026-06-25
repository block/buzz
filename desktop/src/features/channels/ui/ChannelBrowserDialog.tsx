import * as React from "react";
import { Compass, Search, X, type LucideIcon } from "lucide-react";

import type { Channel } from "@/shared/api/types";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import {
  MODAL_SEARCH_INPUT_CLASS,
  MODAL_SEARCH_SHELL_CLASS,
} from "@/shared/ui/modalSearchStyles";
import { Tabs, TabsList, TabsTrigger } from "@/shared/ui/tabs";

const BROWSE_CHANNELS_SHORTCUT_HINT = "\u21E7\u2318O";
type BrowserTab = "all" | "joined" | "archived";

function BrowseState({
  icon: Icon,
  title,
  description,
}: {
  icon: LucideIcon;
  title: string;
  description: string;
}) {
  return (
    <div className="flex flex-col items-center justify-center px-6 py-16 text-center">
      <div className="flex h-12 w-12 items-center justify-center rounded-2xl bg-primary/10 text-primary">
        <Icon className="h-4 w-4" />
      </div>
      <p className="mt-4 text-base font-semibold tracking-tight">{title}</p>
      <p className="mt-2 max-w-md text-sm text-muted-foreground">
        {description}
      </p>
    </div>
  );
}

type ChannelBrowserDialogProps = {
  channels: Channel[];
  channelTypeFilter?: "stream" | "forum";
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onJoinChannel: (channelId: string) => Promise<void>;
  onSelectChannel: (channelId: string) => void;
};

export function ChannelBrowserDialog({
  channels,
  channelTypeFilter,
  open,
  onOpenChange,
  onJoinChannel,
  onSelectChannel,
}: ChannelBrowserDialogProps) {
  const [query, setQuery] = React.useState("");
  const [activeTab, setActiveTab] = React.useState<BrowserTab>("all");
  const [selectedIndex, setSelectedIndex] = React.useState<number | null>(null);
  const [joiningChannelId, setJoiningChannelId] = React.useState<string | null>(
    null,
  );
  const inputRef = React.useRef<HTMLInputElement>(null);
  const tabListRef = React.useRef<HTMLDivElement>(null);
  const tabTriggerRefs = React.useRef<
    Record<BrowserTab, HTMLButtonElement | null>
  >({
    all: null,
    joined: null,
    archived: null,
  });
  const [tabIndicator, setTabIndicator] = React.useState({
    left: 0,
    width: 0,
  });
  const deferredQuery = React.useDeferredValue(query.trim().toLowerCase());

  const isForumMode = channelTypeFilter === "forum";
  const browseTitle = isForumMode ? "Browse Forums" : "Browse Channels";
  const searchPlaceholder = isForumMode
    ? "Search forums by name or description"
    : "Search channels by name or description";
  const entityLabel = isForumMode ? "forum" : "channel";

  const matchingChannels = React.useMemo(() => {
    const filtered = channels.filter(
      (channel) =>
        channel.channelType !== "dm" &&
        (channel.archivedAt
          ? channel.isMember
          : channel.visibility === "open" || channel.isMember) &&
        (channelTypeFilter ? channel.channelType === channelTypeFilter : true),
    );

    if (deferredQuery.length === 0) {
      return filtered;
    }

    return filtered.filter(
      (channel) =>
        channel.name.toLowerCase().includes(deferredQuery) ||
        channel.description.toLowerCase().includes(deferredQuery),
    );
  }, [channels, channelTypeFilter, deferredQuery]);

  const currentChannels = React.useMemo(
    () => matchingChannels.filter((channel) => channel.archivedAt === null),
    [matchingChannels],
  );

  const joinedChannels = React.useMemo(
    () => currentChannels.filter((channel) => channel.isMember),
    [currentChannels],
  );

  const archivedChannels = React.useMemo(
    () => matchingChannels.filter((channel) => channel.archivedAt !== null),
    [matchingChannels],
  );

  const visibleChannels =
    activeTab === "archived"
      ? archivedChannels
      : activeTab === "joined"
        ? joinedChannels
        : matchingChannels;

  const orderedVisibleChannels = React.useMemo(
    () => [
      ...visibleChannels.filter((channel) => !channel.isMember),
      ...visibleChannels.filter((channel) => channel.isMember),
    ],
    [visibleChannels],
  );

  const allTabLabel = isForumMode ? "All forums" : "All channels";

  const updateTabIndicator = React.useCallback(() => {
    const list = tabListRef.current;
    const trigger = tabTriggerRefs.current[activeTab];

    if (!open || !list || !trigger) {
      return;
    }

    const nextIndicator = {
      left: trigger.offsetLeft,
      width: trigger.offsetWidth,
    };

    setTabIndicator((current) =>
      Math.abs(current.left - nextIndicator.left) < 0.5 &&
      Math.abs(current.width - nextIndicator.width) < 0.5
        ? current
        : nextIndicator,
    );
  }, [activeTab, open]);

  React.useLayoutEffect(() => {
    updateTabIndicator();

    if (!open) {
      return;
    }

    let isCancelled = false;
    const updateIfActive = () => {
      if (!isCancelled) {
        updateTabIndicator();
      }
    };
    const frameId = window.requestAnimationFrame(updateIfActive);
    const observer = new ResizeObserver(updateTabIndicator);
    const list = tabListRef.current;

    void document.fonts.ready.then(updateIfActive);

    if (list) {
      observer.observe(list);
    }

    for (const trigger of Object.values(tabTriggerRefs.current)) {
      if (trigger) {
        observer.observe(trigger);
      }
    }

    return () => {
      isCancelled = true;
      window.cancelAnimationFrame(frameId);
      observer.disconnect();
    };
  }, [open, updateTabIndicator]);

  React.useEffect(() => {
    if (!open) {
      setQuery("");
      setActiveTab("all");
      setSelectedIndex(null);
      setJoiningChannelId(null);
      return;
    }
  }, [open]);

  React.useEffect(() => {
    setSelectedIndex((current) => {
      if (current === null || orderedVisibleChannels.length === 0) {
        return null;
      }

      return Math.min(current, orderedVisibleChannels.length - 1);
    });
  }, [orderedVisibleChannels]);

  async function handleJoin(channelId: string) {
    setJoiningChannelId(channelId);

    try {
      await onJoinChannel(channelId);
      onOpenChange(false);
      onSelectChannel(channelId);
    } catch {
      setJoiningChannelId(null);
    }
  }

  function handleSelect(channel: Channel) {
    onOpenChange(false);
    onSelectChannel(channel.id);
  }

  const selectedItem =
    selectedIndex !== null ? orderedVisibleChannels[selectedIndex] : undefined;
  const emptyTitle =
    deferredQuery.length > 0
      ? `No ${entityLabel}s match your search`
      : activeTab === "archived"
        ? `No archived ${entityLabel}s`
        : activeTab === "joined"
          ? `No joined ${entityLabel}s`
          : `No ${entityLabel}s to browse`;
  const emptyDescription =
    deferredQuery.length > 0
      ? "Try a different name or keyword."
      : activeTab === "archived"
        ? `Archived ${entityLabel}s you have joined will appear here.`
        : activeTab === "joined"
          ? `${entityLabel[0].toUpperCase()}${entityLabel.slice(1)}s you join will appear here.`
          : `All open ${entityLabel}s are available in the sidebar. Create a new ${entityLabel} to get started.`;

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent
        aria-describedby={undefined}
        className="gap-0 overflow-hidden border-0 px-6 pb-0 pt-6"
        data-testid={
          isForumMode ? "forum-browser-dialog" : "channel-browser-dialog"
        }
        onOpenAutoFocus={(event) => {
          event.preventDefault();
          inputRef.current?.focus({ preventScroll: true });
        }}
        showCloseButton={false}
      >
        <DialogHeader className="space-y-0 pb-5">
          <div className="flex items-center justify-between gap-4">
            <DialogTitle>{browseTitle}</DialogTitle>
            <DialogClose className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors duration-150 ease-out hover:bg-accent hover:text-accent-foreground focus:outline-hidden focus:ring-1 focus:ring-ring">
              <X className="h-4 w-4" />
              <span className="sr-only">Close</span>
            </DialogClose>
          </div>
          <label
            className={MODAL_SEARCH_SHELL_CLASS}
            htmlFor="channel-browser-search"
          >
            <Search className="h-4 w-4 shrink-0 text-muted-foreground/55 transition-colors duration-150 ease-out group-hover/search:text-muted-foreground group-focus-within/search:text-foreground" />
            <input
              autoCapitalize="none"
              autoCorrect="off"
              className={MODAL_SEARCH_INPUT_CLASS}
              data-testid="channel-browser-search"
              id="channel-browser-search"
              onChange={(event) => {
                setQuery(event.target.value);
                setSelectedIndex(null);
              }}
              onKeyDown={(event) => {
                if (
                  event.key === "ArrowDown" &&
                  orderedVisibleChannels.length > 0
                ) {
                  event.preventDefault();
                  setSelectedIndex((current) =>
                    current === null
                      ? 0
                      : Math.min(
                          current + 1,
                          orderedVisibleChannels.length - 1,
                        ),
                  );
                  return;
                }

                if (
                  event.key === "ArrowUp" &&
                  orderedVisibleChannels.length > 0
                ) {
                  event.preventDefault();
                  setSelectedIndex((current) =>
                    current === null
                      ? orderedVisibleChannels.length - 1
                      : Math.max(current - 1, 0),
                  );
                  return;
                }

                if (
                  event.key === "Enter" &&
                  !event.nativeEvent.isComposing &&
                  orderedVisibleChannels.length > 0
                ) {
                  event.preventDefault();
                  handleSelect(selectedItem ?? orderedVisibleChannels[0]);
                }
              }}
              placeholder={searchPlaceholder}
              ref={inputRef}
              spellCheck={false}
              type="text"
              value={query}
            />
            <span
              className={`hidden shrink-0 text-xs text-muted-foreground/50 transition-opacity duration-150 ease-out group-focus-within/search:opacity-0 sm:block ${
                query.length > 0 ? "opacity-0" : "opacity-100"
              }`}
            >
              {BROWSE_CHANNELS_SHORTCUT_HINT}
            </span>
          </label>
        </DialogHeader>

        <div className="h-[min(60vh,30rem)] overflow-hidden">
          <div className="flex h-full flex-col">
            <Tabs
              className="shrink-0"
              onValueChange={(value) => {
                setActiveTab(value as BrowserTab);
                setSelectedIndex(null);
              }}
              value={activeTab}
            >
              <TabsList
                className="relative h-auto w-full justify-start gap-6 rounded-none border-b border-border/70 bg-transparent p-0 text-muted-foreground"
                ref={tabListRef}
              >
                <span
                  aria-hidden="true"
                  className="pointer-events-none absolute bottom-[-1px] left-0 h-0.5 w-px origin-left rounded-full bg-foreground opacity-0 transition-[transform,opacity] duration-[180ms] ease-[cubic-bezier(0.23,1,0.32,1)] motion-reduce:transition-none data-[ready=true]:opacity-100"
                  data-ready={tabIndicator.width > 0}
                  data-testid="channel-browser-tab-indicator"
                  style={{
                    transform: `translate3d(${tabIndicator.left}px, 0, 0) scaleX(${tabIndicator.width})`,
                  }}
                />
                <TabsTrigger
                  className="rounded-none border-b-2 border-transparent bg-transparent px-0 py-2 text-sm font-medium shadow-none transition-colors duration-150 ease-out data-[state=active]:bg-transparent data-[state=active]:text-foreground data-[state=active]:shadow-none"
                  ref={(element) => {
                    tabTriggerRefs.current.all = element;
                  }}
                  value="all"
                >
                  {allTabLabel}
                </TabsTrigger>
                <TabsTrigger
                  className="rounded-none border-b-2 border-transparent bg-transparent px-0 py-2 text-sm font-medium shadow-none transition-colors duration-150 ease-out data-[state=active]:bg-transparent data-[state=active]:text-foreground data-[state=active]:shadow-none"
                  ref={(element) => {
                    tabTriggerRefs.current.joined = element;
                  }}
                  value="joined"
                >
                  Joined
                </TabsTrigger>
                <TabsTrigger
                  className="rounded-none border-b-2 border-transparent bg-transparent px-0 py-2 text-sm font-medium shadow-none transition-colors duration-150 ease-out data-[state=active]:bg-transparent data-[state=active]:text-foreground data-[state=active]:shadow-none"
                  ref={(element) => {
                    tabTriggerRefs.current.archived = element;
                  }}
                  value="archived"
                >
                  Archived
                </TabsTrigger>
              </TabsList>
            </Tabs>

            <div className="min-h-0 flex-1 overflow-y-auto pb-6 pt-4">
              {orderedVisibleChannels.length === 0 ? (
                <BrowseState
                  description={emptyDescription}
                  icon={deferredQuery.length > 0 ? Search : Compass}
                  title={emptyTitle}
                />
              ) : (
                <div className="overflow-hidden rounded-xl border border-border/70 bg-background/70 shadow-xs divide-y divide-border/55">
                  {orderedVisibleChannels.map((channel, index) => (
                    <ChannelCard
                      channel={channel}
                      isJoining={joiningChannelId === channel.id}
                      isSelected={index === selectedIndex}
                      key={channel.id}
                      onJoin={
                        !channel.isMember
                          ? () => {
                              void handleJoin(channel.id);
                            }
                          : undefined
                      }
                      onSelect={() => handleSelect(channel)}
                    />
                  ))}
                </div>
              )}
            </div>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function ChannelCard({
  channel,
  isJoining,
  isSelected,
  onJoin,
  onSelect,
}: {
  channel: Channel;
  isJoining: boolean;
  isSelected: boolean;
  onJoin?: () => void;
  onSelect: () => void;
}) {
  const memberLabel = `${channel.memberCount} ${
    channel.memberCount === 1 ? "member" : "members"
  }`;

  return (
    <div
      className={
        isSelected
          ? "group/channel-row flex min-h-16 items-center gap-4 bg-muted/40 px-4 py-3 transition-colors duration-150 ease-out"
          : "group/channel-row flex min-h-16 items-center gap-4 px-4 py-3 transition-colors duration-150 ease-out hover:bg-muted/40"
      }
      data-testid={`browse-channel-${channel.name}`}
    >
      <button
        className="min-w-0 flex-1 border-0 bg-transparent p-0 text-left text-foreground outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
        onClick={(event) => {
          event.stopPropagation();
          onSelect();
        }}
        type="button"
      >
        <div className="min-w-0">
          <div className="flex min-w-0 items-center gap-1.5">
            <span className="shrink-0 text-sm font-normal text-muted-foreground">
              #
            </span>
            <p className="min-w-0 truncate text-base font-medium tracking-tight">
              {channel.name}
            </p>
            {channel.archivedAt ? (
              <Badge className="ml-1 shrink-0" variant="warning">
                archived
              </Badge>
            ) : null}
          </div>
          <p className="mt-1 truncate text-sm text-muted-foreground">
            <span>{memberLabel}</span>
            {channel.description ? (
              <>
                <span className="px-1.5">·</span>
                <span title={channel.description}>{channel.description}</span>
              </>
            ) : null}
          </p>
        </div>
      </button>

      {!channel.isMember && onJoin ? (
        <Button
          className={
            isJoining
              ? "shrink-0"
              : "shrink-0 opacity-0 transition-opacity duration-150 ease-out group-hover/channel-row:opacity-100 group-focus-within/channel-row:opacity-100"
          }
          disabled={isJoining}
          onClick={(event) => {
            event.stopPropagation();
            onJoin();
          }}
          size="sm"
          type="button"
          variant="default"
        >
          {isJoining ? "Joining..." : "Join"}
        </Button>
      ) : null}
    </div>
  );
}
