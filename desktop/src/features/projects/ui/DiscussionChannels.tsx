import { Hash } from "lucide-react";
import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useChannelReferences } from "@/features/channels/openChannelDirectory";
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
  mergeOriginDiscussionChannel,
} from "@/features/projects/lib/discussionChannels";
import { selectionItemFromChannel } from "@/features/projects/lib/projectSelection";
import { relativeTime } from "@/features/projects/lib/projectsViewHelpers";
import { useSearchMessagesQuery } from "@/features/search/hooks";
import type { SearchHit } from "@/shared/api/searchTypes";
import { KIND_FORUM_COMMENT, KIND_FORUM_POST } from "@/shared/constants/kinds";
import { cn } from "@/shared/lib/cn";
import { BuzzLoadingState } from "@/shared/ui/BuzzLoadingState";
import { Markdown } from "@/shared/ui/markdown";
import {
  ProjectEntityFacepile,
  ProjectEntityListRow,
} from "./ProjectEntityListRow";
import { ProjectPanelState } from "./ProjectPanelState";
import { useProjectConversationPanel } from "./ProjectConversationPanelContext";

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

/** Hits are sorted newest first, so the first hit per channel is the one a
 * click should land on (and the one worth quoting). */
function useLatestHitByChannel(hits: SearchHit[]) {
  return React.useMemo(() => {
    const byChannel = new Map<string, SearchHit>();
    for (const hit of hits) {
      if (hit.channelId && !byChannel.has(hit.channelId)) {
        byChannel.set(hit.channelId, hit);
      }
    }
    return byChannel;
  }, [hits]);
}

/**
 * Shared row-click behavior: land on the latest matching message — in the
 * side conversation panel when one is mounted — instead of jumping straight
 * to the channel. Forum content opens in place (the panel renders chat
 * threads only), and channels with no quotable hit fall back to plain
 * channel navigation.
 */
function openDiscussionHit({
  channelId,
  goChannel,
  latestHit,
  openSearchHit,
  panel,
}: {
  channelId: string;
  goChannel: (channelId: string) => unknown;
  latestHit: SearchHit | undefined;
  openSearchHit: (hit: SearchHit) => unknown;
  panel: { openConversation: (hit: SearchHit) => void } | null;
}) {
  if (!latestHit) {
    void goChannel(channelId);
    return;
  }
  const opensForum =
    latestHit.kind === KIND_FORUM_POST || latestHit.kind === KIND_FORUM_COMMENT;
  if (!panel || opensForum) {
    void openSearchHit(latestHit);
    return;
  }
  panel.openConversation(latestHit);
}

/** Channel display name, preferring the hit's name, then bounded metadata,
 * then a short id so inaccessible/renamed channels still render something. */
function useChannelNameLookup(channelIds: readonly string[]) {
  const { channelsById } = useChannelReferences(channelIds, {
    enabled: channelIds.length > 0,
  });
  return React.useCallback(
    (id: string, nameFromHit: string | null) =>
      nameFromHit ?? channelsById.get(id)?.name ?? id.slice(0, 8),
    [channelsById],
  );
}

/**
 * "Related Conversations" card for PR, issue, and commit detail views: a
 * bordered block under the body. Each channel gets a name line plus a
 * compact markdown preview of the latest message (same `inbox-preview-markdown`
 * treatment as inbox rows). Tasks and reviews also include the origin
 * channel (`h` tag) as a channel-only row — the tag proves only which
 * channel the entity came from, so no message is quoted or attributed for
 * it. Renders nothing when search and origin are both empty.
 */
export function DiscussedInChannels({
  className,
  entityLabel = "this",
  originChannelId,
  originCreatedAt,
  originPubkey,
  query,
  testId,
}: {
  /** Extra spacing/alignment classes from the call site. */
  className?: string;
  /** How the sentence names the entity, e.g. "this issue". */
  entityLabel?: string;
  originChannelId?: string | null;
  originCreatedAt?: number;
  originPubkey?: string;
  query: string;
  testId?: string;
}) {
  const {
    channels: discussed,
    hits,
    isTruncated,
  } = useDiscussionChannels(query);
  const origin = React.useMemo(
    () =>
      originChannelId && originPubkey && originCreatedAt != null
        ? {
            channelId: originChannelId,
            createdAt: originCreatedAt,
            pubkey: originPubkey,
          }
        : null,
    [originChannelId, originCreatedAt, originPubkey],
  );
  const channels = React.useMemo(
    () => mergeOriginDiscussionChannel(discussed, origin),
    [discussed, origin],
  );
  const { goChannel, openSearchHit } = useAppNavigation();
  const projectConversationPanel = useProjectConversationPanel();
  const [expanded, setExpanded] = React.useState(false);
  const channelIds = React.useMemo(
    () => channels.map((channel) => channel.id),
    [channels],
  );
  const channelName = useChannelNameLookup(channelIds);
  const visible = expanded
    ? channels
    : channels.slice(0, COLLAPSED_MENTION_ROWS);
  const profilesQuery = useUsersBatchQuery(
    visible.flatMap((channel) => channel.participants),
    { enabled: visible.length > 0 },
  );
  const profiles = profilesQuery.data?.profiles;
  // The origin channel has no quotable hit: the `h` tag proves only the
  // channel, so its row navigates to the channel without claiming any
  // particular message.
  const latestHitByChannel = useLatestHitByChannel(hits);
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
        Related Conversations
      </h4>
      <div className="divide-y divide-border/40">
        {visible.map((channel) => {
          const latestHit = latestHitByChannel.get(channel.id);
          const name = channelName(channel.id, channel.name);
          const openConversation = () =>
            openDiscussionHit({
              channelId: channel.id,
              goChannel,
              latestHit,
              openSearchHit,
              panel: projectConversationPanel,
            });
          return (
            <div
              className="group relative flex w-full min-w-0 items-start gap-2.5 px-3 py-2 transition-colors hover:bg-muted/30"
              data-testid="discussion-mention-row"
              key={channel.id}
            >
              <button
                aria-label={
                  latestHit
                    ? `Open conversation in #${name}`
                    : `Open channel #${name}`
                }
                className="absolute inset-0"
                onClick={openConversation}
                title={
                  latestHit
                    ? `Open the latest conversation in #${name}`
                    : `Open #${name}`
                }
                type="button"
              />
              <span className="relative z-10 pt-0.5">
                <ProjectEntityFacepile
                  interactive
                  participants={channel.participants}
                  profiles={profiles}
                />
              </span>
              <span className="pointer-events-none relative z-10 min-w-0 flex-1 text-sm">
                <span className="block truncate">
                  <DiscussionNameList
                    participants={channel.participants}
                    profiles={profiles}
                  />
                  <span className="text-muted-foreground">
                    {" "}
                    {latestHit
                      ? `discussed ${entityLabel} in`
                      : `created ${entityLabel} from`}{" "}
                  </span>
                  <button
                    className="pointer-events-auto font-medium text-foreground hover:underline"
                    onClick={() => void goChannel(channel.id)}
                    title={`Open #${name}`}
                    type="button"
                  >
                    #{name}
                  </button>
                  <span className="text-xs text-muted-foreground">
                    {" "}
                    · {relativeTime(channel.lastActivityAt)}
                  </span>
                </span>
                {latestHit ? (
                  <span className="block text-xs text-muted-foreground">
                    <DiscussionMessagePreview content={latestHit.content} />
                  </span>
                ) : null}
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
          Show {hiddenCount} more{" "}
          {hiddenCount === 1 ? "conversation" : "conversations"}
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

/** Compact markdown preview matching inbox list rows: first block only,
 * clamped, non-interactive so the overlay click still opens the thread. */
function DiscussionMessagePreview({ content }: { content: string }) {
  return (
    <Markdown
      className="inbox-preview-markdown mt-0.5 text-inherit leading-6"
      content={discussionSnippet(content)}
      interactive={false}
    />
  );
}

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
                className="pointer-events-auto font-medium text-foreground hover:underline"
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

/**
 * Full-width channel list for the workspace "Channels" tab: every channel
 * where the repository (or its PRs/issues) is linked in chat, with the
 * people who discussed it there. Clicking a row opens the latest matching
 * conversation in the side panel (whose header still jumps to the channel)
 * rather than leaving the project view.
 */
export function DiscussionChannelsPanel({
  query,
  repositoryName,
}: {
  query: string;
  repositoryName: string;
}) {
  const { channels, hits, isLoading, isTruncated } =
    useDiscussionChannels(query);
  const { goChannel, openSearchHit } = useAppNavigation();
  const projectConversationPanel = useProjectConversationPanel();
  const latestHitByChannel = useLatestHitByChannel(hits);
  const channelIds = React.useMemo(
    () => channels.map((channel) => channel.id),
    [channels],
  );
  const channelName = useChannelNameLookup(channelIds);

  const profilesQuery = useUsersBatchQuery(
    channels.flatMap((channel) => channel.participants),
    { enabled: channels.length > 0 },
  );
  const profiles = profilesQuery.data?.profiles;

  if (isLoading) {
    return <BuzzLoadingState label="Loading channel discussions" />;
  }
  if (channels.length === 0) {
    return (
      <ProjectPanelState
        className="px-4"
        description="Paste this repository, review, or task link in a channel and it will appear here."
        testId="project-discussion-channels-panel"
        title="No linked channels yet"
      />
    );
  }

  const rangeItems = channels.map((channel) =>
    selectionItemFromChannel({
      channelId: channel.id,
      people: channel.participants,
      title: `#${channelName(channel.id, channel.name)}`,
    }),
  );

  return (
    <div className="px-4" data-testid="project-discussion-channels-panel">
      <ul data-testid="discussion-channels">
        {channels.map((channel) => {
          const name = channelName(channel.id, channel.name);
          const latestHit = latestHitByChannel.get(channel.id);
          return (
            <li className="relative" key={channel.id}>
              <ProjectEntityListRow
                affiliation={repositoryName}
                affiliationTestId="project-channel-repository"
                count={channel.messageCount}
                countSuffix={isTruncated ? "+" : undefined}
                countTestId="project-channel-message-count"
                countTitle={`${channel.messageCount}${isTruncated ? "+" : ""} ${
                  channel.messageCount === 1 ? "message" : "messages"
                }`}
                dateSeconds={channel.lastActivityAt}
                dateTestId="project-channel-row-date"
                icon={<Hash className="h-3.5 w-3.5 text-muted-foreground/70" />}
                onClick={() =>
                  openDiscussionHit({
                    channelId: channel.id,
                    goChannel,
                    latestHit,
                    openSearchHit,
                    panel: projectConversationPanel,
                  })
                }
                people={channel.participants}
                peopleTestId="project-channel-participants"
                profiles={profiles}
                selection={{
                  item: selectionItemFromChannel({
                    channelId: channel.id,
                    people: channel.participants,
                    title: `#${name}`,
                  }),
                  rangeItems,
                }}
                testId="project-channel-row"
                title={`#${name}`}
                titleAttr={
                  latestHit
                    ? `Open the latest conversation in #${name}`
                    : `Open #${name}`
                }
              />
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
