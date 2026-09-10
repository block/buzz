import * as React from "react";
import { useWorkingChannels } from "@/features/agents/agentWorkingSignal";
import { TypingDots } from "@/features/messages/ui/TypingDots";
import {
  PULSE_WORKSPACE_KEYS,
  CLEAR_WORKSPACE_PANELS,
  type PulseView,
} from "../lib/workspaceNavigation";
import { Inbox, Search, MessageCircle, Users } from "lucide-react";
import { allowNavigation } from "@/app/navigation/navigationGuard";
import { buildDirectMessageIntro } from "@/features/channels/lib/dmParticipantDisplay";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import type { Channel } from "@/shared/api/types";
import { useHistorySearchState } from "@/shared/hooks/useHistorySearchState";
import { usePresenceQuery } from "@/features/presence/hooks";
import {
  DEFAULT_HOVER_PROFILE_STATUS_GEOMETRY,
  ProfileAvatarWithStatus,
  scaleProfileAvatarStatusGeometry,
} from "@/features/profile/ui/ProfileAvatarWithStatus";
import { normalizePubkey } from "@/shared/lib/pubkey";

import { PulseChannelAvatar } from "./PulseChannelAvatar";
import { PulseChannelDetail } from "./PulseChannelDetail";
import {
  PULSE_STATUS_DOT_CLASS,
  PulseUnreadDot,
  usePulseUnreadChannels,
} from "./PulseUnreadDot";
import {
  PULSE_CONVERSATION_KEYS,
  CLEAR_CONVERSATION_PANELS,
} from "../lib/pulsePanelState";

const AVATAR_SIZE = 28;
const AVATAR_STATUS_GEOMETRY = scaleProfileAvatarStatusGeometry(
  DEFAULT_HOVER_PROFILE_STATUS_GEOMETRY,
  AVATAR_SIZE,
);

const SPLIT_VIEW_SEARCH_KEYS = [
  "feed",
  ...PULSE_CONVERSATION_KEYS,
  ...PULSE_WORKSPACE_KEYS,
] as const;

export function PulseConversationSplitView({
  channels,
  currentPubkey,
  search = "",
  selectionKey,
  label,
  testPrefix,
  allMessages,
  navigation,
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
  navigation?: {
    view: PulseView;
    onSelectView: (view: PulseView) => void;
  };
}) {
  const isUnread = usePulseUnreadChannels();
  const workingChannels = useWorkingChannels();
  const workingAgents = React.useMemo(
    () =>
      new Set(
        workingChannels.flatMap((channel) =>
          channel.agentPubkeys.map(normalizePubkey),
        ),
      ),
    [workingChannels],
  );
  const { values, applyPatch } = useHistorySearchState(SPLIT_VIEW_SEARCH_KEYS);
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
  const presence = usePresenceQuery(pubkeys, { enabled: pubkeys.length > 0 });
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
    navigation && navigation.view !== "conversation"
      ? null
      : (channels.find((channel) => channel.id === values[selectionKey]) ??
        (allMessages ? null : visibleRows[0]?.channel) ??
        null);
  const selectedId = selected?.id;
  React.useEffect(() => {
    if (!values[selectionKey] && selectedId) {
      applyPatch({ [selectionKey]: selectedId }, { replace: true });
    }
  }, [values[selectionKey], selectedId, applyPatch, selectionKey]);
  const selectConversation = (channel: Channel | null) => {
    const channelId = channel?.id ?? null;
    if (
      (!navigation || navigation.view === "conversation") &&
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
    applyPatch({
      ...CLEAR_CONVERSATION_PANELS,
      ...CLEAR_WORKSPACE_PANELS,
      [selectionKey]: channelId,
      ...(navigation ? { feed: "conversation" } : {}),
    });
  };

  return (
    <div
      className="flex min-h-0 min-w-0 flex-1"
      data-testid={`${testPrefix}-view`}
    >
      <nav
        aria-label={label}
        className="w-[220px] min-w-0 shrink-0 overflow-y-auto border-r border-border/50 p-2"
        data-testid={`${testPrefix}-list`}
      >
        {navigation &&
          (
            [
              { id: "search", label: "Search", icon: Search },
              { id: "all", label: "For you", icon: Inbox },
            ] as const
          ).map(({ id, label, icon: Icon }) => (
            <button
              key={id}
              type="button"
              aria-current={navigation.view === id ? "true" : undefined}
              onClick={() => navigation.onSelectView(id)}
              className={`mb-1 flex w-full items-center gap-3 rounded-xl px-3 py-3 text-left text-sm focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring ${navigation.view === id ? "bg-muted/70 font-semibold" : "hover:bg-muted/35"}`}
            >
              <span
                aria-hidden
                className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-muted text-muted-foreground"
              >
                <Icon className="h-4 w-4" />
              </span>
              {label}
            </button>
          ))}
        {allMessages && (
          <>
            <button
              type="button"
              aria-current={
                !selected && (!navigation || navigation.view === "conversation")
                  ? "true"
                  : undefined
              }
              onClick={() => selectConversation(null)}
              className={`mb-1 flex w-full items-center gap-3 rounded-xl px-3 py-3 text-left text-sm focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring ${!selected && (!navigation || navigation.view === "conversation") ? "bg-muted/70 font-semibold" : "hover:bg-muted/35"}`}
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
            const working =
              row.channel.channelType === "dm" &&
              row.participants.some((person) =>
                workingAgents.has(normalizePubkey(person.pubkey)),
              );
            const unread = isUnread(row.channel.id);
            return (
              <button
                type="button"
                key={row.channel.id}
                data-testid={`${testPrefix}-${row.channel.id}`}
                data-channel-name={row.channel.name}
                aria-description={
                  [working && "Agent working", unread && "Unread messages"]
                    .filter(Boolean)
                    .join(". ") || undefined
                }
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
                  <ProfileAvatarWithStatus
                    avatarUrl={participant?.avatarUrl ?? null}
                    label={row.name}
                    avatarClassName="text-xs"
                    className="h-7 w-7 shrink-0"
                    geometry={AVATAR_STATUS_GEOMETRY}
                    size={AVATAR_SIZE}
                    status={
                      presence.data?.[
                        normalizePubkey(participant?.pubkey ?? "")
                      ]
                    }
                    statusTestId="pulse-conversation-presence"
                    shape={participant?.isAgent ? "squircle" : "circle"}
                  />
                )}
                <span
                  className={`min-w-0 truncate text-sm ${selected?.id === row.channel.id ? "font-semibold" : ""}`}
                >
                  {row.name}
                </span>
                {working ? (
                  <span
                    aria-hidden="true"
                    data-testid="pulse-working-dots"
                    className="ml-auto flex shrink-0 items-center"
                  >
                    <TypingDots
                      className="gap-[3px] [--typing-dot-lift:-2px]"
                      dotClassName={PULSE_STATUS_DOT_CLASS}
                    />
                  </span>
                ) : unread ? (
                  <PulseUnreadDot />
                ) : null}
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
          data-testid={
            navigation?.view === "search"
              ? "pulse-search-feed"
              : navigation?.view === "all"
                ? "pulse-briefing-feed"
                : "pulse-all-messages-feed"
          }
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
