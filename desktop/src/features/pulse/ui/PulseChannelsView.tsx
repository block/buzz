import type * as React from "react";
import { allowNavigation } from "@/app/navigation/navigationGuard";
import type { Channel } from "@/shared/api/types";
import { useHistorySearchState } from "@/shared/hooks/useHistorySearchState";
import {
  CLEAR_CONVERSATION_PANELS,
  PULSE_CONVERSATION_KEYS,
} from "../lib/pulsePanelState";
import { PulseChannelAvatar } from "./PulseChannelAvatar";
import { PulseChannelDetail } from "./PulseChannelDetail";
import { PulseUnreadDot, usePulseUnreadChannels } from "./PulseUnreadDot";

function rowClass(selected: boolean) {
  return `mb-1 flex w-full items-center gap-3 rounded-xl px-3 py-3 text-left text-sm focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring ${selected ? "bg-muted/70 font-semibold" : "hover:bg-muted/35"}`;
}

export function PulseChannelsView({
  channels,
  search,
  scrollRef,
  children,
}: {
  channels: Channel[];
  search: string;
  scrollRef: React.RefCallback<HTMLDivElement>;
  children: React.ReactNode;
}) {
  const isUnread = usePulseUnreadChannels();
  const { values, applyPatch } = useHistorySearchState(PULSE_CONVERSATION_KEYS);
  const memberChannels = channels.filter(
    (channel) => channel.channelType !== "dm",
  );
  const selected = memberChannels.find(
    (channel) => channel.id === values.channel,
  );
  const query = search.trim().toLocaleLowerCase();
  const visible = memberChannels.filter(
    (channel) =>
      !query ||
      `${channel.name} ${channel.topic ?? ""} ${channel.description}`
        .toLocaleLowerCase()
        .includes(query),
  );
  const selectChannel = (channelId: string | null) => {
    if (
      !allowNavigation({
        kind: "route",
        href: `/pulse?feed=channel${channelId ? `&channel=${channelId}` : ""}`,
      })
    )
      return;
    applyPatch({ ...CLEAR_CONVERSATION_PANELS, channel: channelId });
  };
  return (
    <div
      className="flex min-h-0 min-w-0 flex-1"
      data-testid="pulse-channels-view"
    >
      <nav
        aria-label="Channels"
        className="w-[220px] min-w-0 shrink-0 overflow-y-auto border-r border-border/50 p-2"
        data-testid="pulse-channels-list"
      >
        <button
          type="button"
          aria-current={!selected ? "true" : undefined}
          onClick={() => selectChannel(null)}
          className={rowClass(!selected)}
        >
          <PulseChannelAvatar />
          All channels
        </button>
        <div className="my-2 border-t border-border/40" />
        {visible.map((channel) => {
          return (
            <button
              type="button"
              key={channel.id}
              data-channel-name={channel.name}
              aria-description={
                isUnread(channel.id) ? "Unread messages" : undefined
              }
              aria-current={selected?.id === channel.id ? "true" : undefined}
              onClick={() => selectChannel(channel.id)}
              className={rowClass(selected?.id === channel.id)}
            >
              <PulseChannelAvatar channel={channel} />
              <span className="truncate">{channel.name}</span>
              {isUnread(channel.id) && <PulseUnreadDot />}
            </button>
          );
        })}
        {!visible.length && (
          <p className="px-3 py-5 text-sm text-muted-foreground">
            {query ? "No matching channels" : "No channels yet"}
          </p>
        )}
      </nav>
      {selected ? (
        <div
          className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden"
          data-testid="pulse-channel-detail"
          data-channel-id={selected.id}
        >
          <PulseChannelDetail channel={selected} />
        </div>
      ) : (
        <div
          ref={scrollRef}
          className="min-h-0 min-w-0 flex-1 overflow-y-auto"
          data-testid="pulse-all-channels-feed"
        >
          {children}
        </div>
      )}
    </div>
  );
}
