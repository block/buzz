import * as React from "react";

import { parseSubChannelName } from "@/features/dev-mode/lib/subChannels";
import { DevSpotlightText } from "@/features/dev-mode/ui/DevSpotlightText";
import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";

/**
 * Tab strip across the top of an open channel: `main` plus one tab per
 * sub-channel the user can see (surfaced to users as "tabs"). Parents can
 * carry hundreds, so the strip scrolls horizontally instead of wrapping;
 * the active tab scrolls itself into view. ⇧⌘[/⇧⌘] cycle through tabs;
 * ⌘1–⌘9 jump straight to one.
 */
export function DevChannelTabs({
  main,
  subs,
  activeId,
  unreadChannelIds,
  workingChannelIds,
  highPriorityChannelIds,
  blockedChannelIds,
  sendFailureChannelIds,
  onSelect,
  onNewSubChannel,
}: {
  main: Channel;
  subs: Channel[];
  activeId: string;
  unreadChannelIds: ReadonlySet<string>;
  workingChannelIds: ReadonlySet<string>;
  highPriorityChannelIds: ReadonlySet<string>;
  blockedChannelIds: ReadonlySet<string>;
  sendFailureChannelIds: ReadonlySet<string>;
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
    // An active tab keeps its dot while an unread thread remains inside it —
    // viewing the tab clears top-level posts, not collapsed thread replies.
    const isUnread = unreadChannelIds.has(channel.id);
    const isHighPriority = highPriorityChannelIds.has(channel.id);
    const isBlocked = blockedChannelIds.has(channel.id);
    const isWorking = workingChannelIds.has(channel.id);
    const hasSendFailure = sendFailureChannelIds.has(channel.id);
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
        <span
          className={cn(
            "whitespace-nowrap",
            isUnread && "font-semibold",
            // Mirror the navigator: a mention lights the tab label itself.
            isUnread && isBlocked
              ? "text-destructive"
              : isUnread && isHighPriority
                ? "text-warning"
                : null,
          )}
        >
          {isWorking ? <DevSpotlightText text={label} /> : label}
        </span>
        {hasSendFailure ? (
          <span
            data-testid="dev-mode-send-failure"
            role="img"
            aria-label="message failed to send"
            className="text-xs font-semibold leading-none text-destructive"
          >
            !
          </span>
        ) : null}
        {isUnread ? (
          <span
            aria-label={
              isBlocked ? "blocked" : isHighPriority ? "mentioned" : "unread"
            }
            className={cn(
              "text-3xs leading-none",
              isBlocked
                ? "text-destructive"
                : isHighPriority
                  ? "text-warning"
                  : "text-muted-foreground/60",
            )}
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
        aria-label={`New tab in # ${main.name}`}
        className="shrink-0 cursor-pointer px-2.5 py-1 text-xs text-muted-foreground/60 hover:text-foreground"
        data-testid="dev-mode-new-tab"
        onClick={onNewSubChannel}
        title="⌘⇧T"
        type="button"
      >
        + tab
      </button>
    </div>
  );
}
