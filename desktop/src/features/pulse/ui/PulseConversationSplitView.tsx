import * as React from "react";
import { MessageCircle, Users } from "lucide-react";
import { allowNavigation } from "@/app/navigation/navigationGuard";
import { buildDirectMessageIntro } from "@/features/channels/lib/dmParticipantDisplay";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import type { Channel } from "@/shared/api/types";
import { useHistorySearchState } from "@/shared/hooks/useHistorySearchState";
import { UserAvatar } from "@/shared/ui/UserAvatar";

import { PulseChannelAvatar } from "./PulseChannelAvatar";
import { PulseChannelDetail } from "./PulseChannelDetail";
import {
  PULSE_CONVERSATION_KEYS,
  CLEAR_CONVERSATION_PANELS,
} from "../lib/pulsePanelState";

export function PulseConversationSplitView({
  channels,
  currentPubkey,
  search = "",
  selectionKey,
  label,
  testPrefix,
  allMessages,
}: {
  channels: Channel[];
  currentPubkey?: string;
  search?: string;
  selectionKey: "dm" | "conversation";
  label: string;
  testPrefix: string;
  allMessages?: {
    content: React.ReactNode;
    scrollRef: React.RefCallback<HTMLDivElement>;
  };
}) {
  const { values, applyPatch } = useHistorySearchState(PULSE_CONVERSATION_KEYS);
  const pubkeys = React.useMemo(
    () => [
      ...new Set(
        channels
          .filter((c) => c.channelType === "dm")
          .flatMap((c) => c.participantPubkeys),
      ),
    ],
    [channels],
  );
  const profiles = useUsersBatchQuery(pubkeys, { enabled: pubkeys.length > 0 });
  const rows = React.useMemo(() => {
    return channels.map((channel) => {
      const intro =
        channel.channelType === "dm"
          ? buildDirectMessageIntro({
              channel,
              currentPubkey,
              profiles: profiles.data?.profiles,
            })
          : null;
      return {
        channel,
        name: intro?.displayName || channel.name,
        participants: intro?.participants ?? [],
      };
    });
  }, [channels, currentPubkey, profiles.data]);
  const query = search.trim().toLocaleLowerCase();
  const visibleRows = rows.filter(
    (row) => !query || row.name.toLocaleLowerCase().includes(query),
  );
  const selected =
    channels.find((channel) => channel.id === values[selectionKey]) ??
    (allMessages ? null : visibleRows[0]?.channel) ??
    null;
  const selectedId = selected?.id;
  React.useEffect(() => {
    if (!values[selectionKey] && selectedId) {
      applyPatch({ [selectionKey]: selectedId }, { replace: true });
    }
  }, [values[selectionKey], selectedId, applyPatch, selectionKey]);
  const selectConversation = (channel: Channel | null) => {
    const channelId = channel?.id ?? null;
    if (
      channelId === (selected?.id ?? null) &&
      values[selectionKey] === channelId
    )
      return;
    if (
      !allowNavigation({
        kind: "route",
        href: `/pulse?feed=${selectionKey}&${selectionKey}=${channelId ?? ""}`,
      })
    )
      return;
    applyPatch({ ...CLEAR_CONVERSATION_PANELS, [selectionKey]: channelId });
  };

  return (
    <div
      className="flex min-h-0 min-w-0 flex-1"
      data-testid={`${testPrefix}-view`}
    >
      <nav
        aria-label={label}
        className="w-[200px] min-w-0 shrink-0 overflow-y-auto border-r border-border/50 p-2"
        data-testid={`${testPrefix}-list`}
      >
        {allMessages && (
          <>
            <button
              type="button"
              aria-current={!selected ? "true" : undefined}
              onClick={() => selectConversation(null)}
              className={`mb-1 flex w-full items-center gap-3 rounded-xl px-3 py-3 text-left text-sm focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring ${!selected ? "bg-muted/70 font-semibold" : "hover:bg-muted/35"}`}
            >
              <PulseChannelAvatar />
              All messages
            </button>
            <div className="my-2 border-t border-border/40" />
          </>
        )}
        {visibleRows.length ? (
          visibleRows.map((row) => {
            const participant = row.participants[0];
            return (
              <button
                type="button"
                key={row.channel.id}
                data-testid={`${testPrefix}-${row.channel.id}`}
                data-channel-name={row.channel.name}
                aria-label={
                  row.channel.channelType === "dm"
                    ? `Open DM with ${row.name}`
                    : `Open channel ${row.name}`
                }
                aria-current={
                  selected?.id === row.channel.id ? "true" : undefined
                }
                onClick={() => selectConversation(row.channel)}
                className={`mb-1 flex w-full items-center gap-3 rounded-xl px-3 py-3 text-left focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring ${selected?.id === row.channel.id ? "bg-muted/70" : "hover:bg-muted/35"}`}
              >
                {row.channel.channelType !== "dm" ? (
                  <PulseChannelAvatar channel={row.channel} />
                ) : row.participants.length > 1 ? (
                  <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-muted text-muted-foreground">
                    <Users aria-hidden className="h-4 w-4" />
                  </span>
                ) : (
                  <UserAvatar
                    avatarUrl={participant?.avatarUrl ?? null}
                    displayName={row.name}
                    fallbackDelayMs={0}
                    className="!h-7 !w-7 shrink-0"
                    shape={participant?.isAgent ? "squircle" : "circle"}
                  />
                )}
                <span
                  className={`min-w-0 truncate text-sm ${selected?.id === row.channel.id ? "font-semibold" : ""}`}
                >
                  {row.name}
                </span>
              </button>
            );
          })
        ) : (
          <p className="px-4 py-8 text-center text-sm text-muted-foreground">
            {query ? "No matching conversations" : "No conversations yet"}
          </p>
        )}
      </nav>
      {!selected && allMessages ? (
        <div
          ref={allMessages.scrollRef}
          className="min-h-0 min-w-0 flex-1 overflow-y-auto"
          data-testid="pulse-all-messages-feed"
        >
          {allMessages.content}
        </div>
      ) : (
        <div
          className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden"
          data-testid={`${testPrefix}-detail`}
          data-channel-id={selected?.id}
        >
          {selected ? (
            <PulseChannelDetail channel={selected} />
          ) : (
            <div className="flex flex-1 flex-col items-center justify-center gap-3 text-sm text-muted-foreground">
              <MessageCircle aria-hidden className="h-6 w-6" />
              <p>Select a conversation</p>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
