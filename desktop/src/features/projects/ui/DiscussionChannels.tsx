import { Hash } from "lucide-react";
import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useChannelsQuery } from "@/features/channels/hooks";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { UserProfilePopover } from "@/features/profile/ui/UserProfilePopover";
import {
  type DiscussionChannel,
  discussionSnippet,
  groupDiscussionChannels,
} from "@/features/projects/lib/discussionChannels";
import { relativeTime } from "@/features/projects/lib/projectsViewHelpers";
import { useSearchMessagesQuery } from "@/features/search/hooks";
import type { SearchHit } from "@/shared/api/searchTypes";
import { cn } from "@/shared/lib/cn";
import { UserAvatar } from "@/shared/ui/UserAvatar";

// Relay search caps a page at 500. Use the full page and surface a lower-bound
// marker when it fills rather than silently presenting partial totals as exact.
const DISCUSSION_SEARCH_LIMIT = 500;
const COLLAPSED_MENTION_ROWS = 3;

/**
 * Messages (and the channels containing them) that link the entity matched
 * by `query` (see `discussionChannels.ts` for how queries are built).
 * Results cover only channels the viewer can read — the relay authorizes
 * every search hit. Profiles for the discussing authors resolve in the same
 * hook so rows can show names and avatars.
 */
export function useDiscussionChannels(query: string): {
  channels: DiscussionChannel[];
  hits: SearchHit[];
  isLoading: boolean;
  isTruncated: boolean;
} {
  const search = useSearchMessagesQuery(query, {
    limit: DISCUSSION_SEARCH_LIMIT,
  });
  const hits = React.useMemo(
    () =>
      [...(search.data?.hits ?? [])].sort((a, b) => b.createdAt - a.createdAt),
    [search.data],
  );
  const channels = React.useMemo(() => groupDiscussionChannels(hits), [hits]);
  return {
    channels,
    hits,
    isLoading: search.isLoading,
    isTruncated: hits.length >= DISCUSSION_SEARCH_LIMIT,
  };
}

/** Channel display name, preferring the hit's name, then the channel list,
 * then a short id so private/renamed channels still render something. */
function useChannelNameLookup(enabled: boolean) {
  const channelsQuery = useChannelsQuery({ enabled });
  return React.useCallback(
    (id: string, nameFromHit: string | null) =>
      nameFromHit ??
      channelsQuery.data?.find((channel) => channel.id === id)?.name ??
      id.slice(0, 8),
    [channelsQuery.data],
  );
}

/**
 * "Channels" card for PR, issue, and commit detail views: a bordered,
 * softly tinted block with a small header that separates it from the
 * surrounding text. Each channel gets a single truncating line —
 * "Alice, Bob and Carol discussed this in #channel · 2h ago — snippet…" —
 * cut at the card edge regardless of screen width. Clicking the snippet
 * jumps to that message (thread-aware), the same way inbox items do.
 * Renders nothing until at least one channel references the entity, so
 * the detail layout stays unchanged for undiscussed items.
 */
export function DiscussedInChannels({
  className,
  entityLabel = "this",
  query,
  testId,
}: {
  /** Extra spacing/alignment classes from the call site. */
  className?: string;
  /** How the sentence names the entity, e.g. "this issue". */
  entityLabel?: string;
  query: string;
  testId?: string;
}) {
  const { channels, hits, isTruncated } = useDiscussionChannels(query);
  const { goChannel, openSearchHit } = useAppNavigation();
  const [expanded, setExpanded] = React.useState(false);
  const channelName = useChannelNameLookup(channels.length > 0);
  const visible = expanded
    ? channels
    : channels.slice(0, COLLAPSED_MENTION_ROWS);
  const profilesQuery = useUsersBatchQuery(
    visible.flatMap((channel) => channel.participants),
    { enabled: visible.length > 0 },
  );
  const profiles = profilesQuery.data?.profiles;
  // Hits are sorted newest first, so the first hit per channel is the one a
  // click should land on (and the one worth quoting).
  const latestHitByChannel = React.useMemo(() => {
    const byChannel = new Map<string, SearchHit>();
    for (const hit of hits) {
      if (hit.channelId && !byChannel.has(hit.channelId)) {
        byChannel.set(hit.channelId, hit);
      }
    }
    return byChannel;
  }, [hits]);
  if (channels.length === 0) return null;

  const hiddenCount = channels.length - visible.length;

  return (
    <div
      className={cn(
        "min-w-0 overflow-hidden rounded-lg border border-border/60 bg-muted/20",
        className,
      )}
      data-testid={testId}
    >
      <h4 className="border-b border-border/40 px-3 py-1.5 text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
        Channels
      </h4>
      <div className="divide-y divide-border/40">
        {visible.map((channel) => {
          const latestHit = latestHitByChannel.get(channel.id);
          if (!latestHit) return null;
          const name = channelName(channel.id, channel.name);
          return (
            <div
              className="flex w-full min-w-0 items-center gap-2.5 px-3 py-2"
              data-testid="discussion-mention-row"
              key={channel.id}
            >
              <ParticipantFacepile
                interactive
                participants={channel.participants}
                profiles={profiles}
              />
              <span className="min-w-0 flex-1 truncate text-sm">
                <DiscussionNameList
                  participants={channel.participants}
                  profiles={profiles}
                />
                <span className="text-muted-foreground">
                  {" "}
                  discussed {entityLabel} in{" "}
                </span>
                <button
                  className="font-medium text-foreground hover:underline"
                  onClick={() => void goChannel(channel.id)}
                  title={`Open #${name}`}
                  type="button"
                >
                  #{name}
                </button>
                <button
                  className="text-muted-foreground hover:underline"
                  onClick={() => void openSearchHit(latestHit)}
                  title={`Open the latest mention in #${name}`}
                  type="button"
                >
                  <span className="text-xs">
                    {" "}
                    · {relativeTime(channel.lastActivityAt)}
                  </span>
                  <span> — {discussionSnippet(latestHit.content)}</span>
                </button>
              </span>
            </div>
          );
        })}
      </div>
      {hiddenCount > 0 ? (
        <button
          className="w-full border-t border-border/40 px-3 py-1.5 text-left text-xs font-medium text-muted-foreground transition-colors hover:text-foreground"
          onClick={() => setExpanded(true)}
          type="button"
        >
          Show {hiddenCount} more {hiddenCount === 1 ? "channel" : "channels"}
        </button>
      ) : null}
      {isTruncated ? (
        <p className="border-t border-border/40 px-3 py-1.5 text-xs text-muted-foreground">
          Showing mentions from the 500 most recent search results.
        </p>
      ) : null}
    </div>
  );
}

const NAME_LIST_MAX = 3;

/**
 * The "Alice, Bob and Carol" (or "Alice, Bob and 2 others") part of the
 * sentence, with each name opening that person's profile popover. Mirrors
 * the wording of `formatNameList` in `discussionChannels.ts`.
 */
function DiscussionNameList({
  participants,
  profiles,
}: {
  participants: string[];
  profiles: UserProfileLookup | undefined;
}) {
  const showAll = participants.length <= NAME_LIST_MAX;
  const shown = showAll
    ? participants
    : participants.slice(0, NAME_LIST_MAX - 1);
  const others = participants.length - shown.length;
  return (
    <>
      {shown.map((pubkey, index) => {
        const isLast = index === shown.length - 1;
        const separator =
          index === 0 ? null : isLast && others === 0 ? " and " : ", ";
        return (
          <React.Fragment key={pubkey}>
            {separator ? (
              <span className="text-muted-foreground">{separator}</span>
            ) : null}
            <UserProfilePopover pubkey={pubkey} triggerElement="span">
              <button
                className="font-medium text-foreground hover:underline"
                type="button"
              >
                {resolveUserLabel({ profiles, pubkey })}
              </button>
            </UserProfilePopover>
          </React.Fragment>
        );
      })}
      {others > 0 ? (
        <span className="font-medium text-foreground">
          {" "}
          and {others} others
        </span>
      ) : null}
    </>
  );
}

function ParticipantFacepile({
  interactive = false,
  participants,
  profiles,
}: {
  /** Wrap each avatar in a profile popover. Leave off when the facepile is
   * nested inside another button (nested interactive elements are invalid). */
  interactive?: boolean;
  participants: string[];
  profiles: UserProfileLookup | undefined;
}) {
  const shown = participants.slice(0, 4);
  const overflow = participants.length - shown.length;
  return (
    <span className="flex shrink-0 items-center">
      {shown.map((pubkey, index) => {
        const label = resolveUserLabel({ profiles, pubkey });
        if (!interactive) {
          return (
            <UserAvatar
              avatarUrl={profiles?.[pubkey]?.avatarUrl ?? null}
              className={cn(
                "rounded-full ring-2 ring-background",
                index > 0 && "-ml-1.5",
              )}
              displayName={label}
              key={pubkey}
              size="xs"
            />
          );
        }
        return (
          <UserProfilePopover
            key={pubkey}
            pubkey={pubkey}
            triggerElement="span"
          >
            <button
              className={cn("rounded-full", index > 0 && "-ml-1.5")}
              title={label}
              type="button"
            >
              <UserAvatar
                avatarUrl={profiles?.[pubkey]?.avatarUrl ?? null}
                className="rounded-full ring-2 ring-background"
                displayName={label}
                size="xs"
              />
            </button>
          </UserProfilePopover>
        );
      })}
      {overflow > 0 ? (
        <span className="-ml-1.5 flex h-5 w-5 items-center justify-center rounded-full bg-muted text-3xs text-muted-foreground ring-2 ring-background">
          +{overflow}
        </span>
      ) : null}
    </span>
  );
}

/**
 * Full-width channel list for the workspace "Channels" tab: every channel
 * where the repository (or its PRs/issues) is linked in chat, with the
 * people who discussed it there.
 */
export function DiscussionChannelsPanel({ query }: { query: string }) {
  const { channels, isLoading, isTruncated } = useDiscussionChannels(query);
  const { goChannel } = useAppNavigation();
  const channelName = useChannelNameLookup(channels.length > 0);
  const profilesQuery = useUsersBatchQuery(
    channels.flatMap((channel) => channel.participants),
    { enabled: channels.length > 0 },
  );
  const profiles = profilesQuery.data?.profiles;

  if (isLoading) {
    return (
      <p className="px-4 py-6 text-sm text-muted-foreground">
        Searching channel discussions…
      </p>
    );
  }
  if (channels.length === 0) {
    return (
      <p className="px-4 py-6 text-sm text-muted-foreground">
        No channels reference this repository yet. Paste its link (or a PR or
        issue link) in a channel and it will show up here.
      </p>
    );
  }

  return (
    <div>
      <ul
        className="divide-y divide-border/50"
        data-testid="discussion-channels"
      >
        {channels.map((channel) => {
          const name = channelName(channel.id, channel.name);
          const speakers = channel.participants
            .slice(0, 2)
            .map((pubkey) => resolveUserLabel({ profiles, pubkey }));
          const others = channel.participants.length - speakers.length;
          return (
            <li className="relative" key={channel.id}>
              <button
                className="flex w-full min-w-0 items-center gap-2.5 px-4 py-3 text-left transition-colors hover:bg-muted/30"
                onClick={() => void goChannel(channel.id)}
                type="button"
              >
                <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-muted/50">
                  <Hash className="h-4 w-4 text-muted-foreground" />
                </span>
                <span className="min-w-0 flex-1 space-y-1">
                  <span className="block truncate text-sm font-medium text-foreground">
                    #{name}
                  </span>
                  <span className="block truncate text-xs text-muted-foreground">
                    {speakers.join(", ")}
                    {others > 0
                      ? ` and ${others} ${others === 1 ? "other" : "others"}`
                      : ""}{" "}
                    · {channel.messageCount}
                    {isTruncated ? "+" : ""}{" "}
                    {channel.messageCount === 1 ? "message" : "messages"}
                  </span>
                </span>
                <ParticipantFacepile
                  participants={channel.participants}
                  profiles={profiles}
                />
                <span
                  className="hidden w-20 shrink-0 text-right text-xs text-muted-foreground sm:block"
                  data-testid="project-channel-row-date"
                  title={new Date(
                    channel.lastActivityAt * 1_000,
                  ).toLocaleString()}
                >
                  {relativeTime(channel.lastActivityAt)}
                </span>
              </button>
            </li>
          );
        })}
      </ul>
      {isTruncated ? (
        <p className="border-t border-border/50 px-4 py-2 text-xs text-muted-foreground">
          Showing the latest {DISCUSSION_SEARCH_LIMIT} mentions; totals may be
          higher.
        </p>
      ) : null}
    </div>
  );
}
