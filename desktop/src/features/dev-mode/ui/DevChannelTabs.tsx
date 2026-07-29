import * as React from "react";

import { parseSubChannelName } from "@/features/dev-mode/lib/subChannels";
import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";

/**
 * Tab strip across the top of an open channel: `main` plus one tab per
 * sub-channel the user can see. Parents can carry hundreds of subs, so the
 * strip scrolls horizontally instead of wrapping; the active tab scrolls
 * itself into view.
 */
export function DevChannelTabs({
  main,
  subs,
  activeId,
  unreadChannelIds,
  onSelect,
  onNewSubChannel,
}: {
  main: Channel;
  subs: Channel[];
  activeId: string;
  unreadChannelIds: ReadonlySet<string>;
  onSelect: (channelId: string) => void;
  onNewSubChannel: () => void;
}) {
  const scrollActiveIntoView = React.useCallback(
    (node: HTMLButtonElement | null) => {
      node?.scrollIntoView({ block: "nearest", inline: "nearest" });
    },
    [],
  );

  const tab = (channel: Channel, label: string) => {
    const isActive = channel.id === activeId;
    const isUnread = !isActive && unreadChannelIds.has(channel.id);
    return (
      <button
        key={channel.id}
        ref={isActive ? scrollActiveIntoView : undefined}
        className={cn(
          "flex shrink-0 cursor-pointer items-baseline gap-1.5 border-b-2 px-2.5 py-1 text-xs",
          isActive
            ? "border-primary text-foreground"
            : "border-transparent text-muted-foreground hover:text-foreground",
        )}
        data-testid="dev-mode-channel-tab"
        data-active={isActive || undefined}
        onClick={() => onSelect(channel.id)}
        type="button"
      >
        <span className={cn("whitespace-nowrap", isUnread && "font-semibold")}>
          {label}
        </span>
        {isUnread ? (
          <span
            aria-label="unread"
            className="text-3xs leading-none text-primary"
            role="img"
          >
            ●
          </span>
        ) : null}
      </button>
    );
  };

  return (
    <div
      className="flex shrink-0 items-center border-b border-border/60 font-mono"
      data-testid="dev-mode-channel-tabs"
    >
      <div className="scrollbar-none flex min-w-0 flex-1 overflow-x-auto">
        {tab(main, "main")}
        {subs.map((sub) =>
          tab(sub, parseSubChannelName(sub.name)?.subSlug ?? sub.name),
        )}
      </div>
      <button
        aria-label={`New sub-channel of # ${main.name}`}
        className="shrink-0 cursor-pointer px-2.5 py-1 text-xs text-muted-foreground/60 hover:text-foreground"
        data-testid="dev-mode-new-sub-channel"
        onClick={onNewSubChannel}
        type="button"
      >
        + sub
      </button>
    </div>
  );
}
