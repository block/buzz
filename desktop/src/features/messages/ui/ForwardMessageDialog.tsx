import { Search } from "lucide-react";
import * as React from "react";

import { scoreChannelMatch } from "@/features/channels/lib/channelSearchScore";
import { oneToOneDmPubkeys } from "@/features/channels/lib/oneToOneDm";
import { ChannelGlyph } from "@/features/channels/ui/ChannelGlyph";
import { useIsArchivedPredicate } from "@/features/identity-archive/hooks";
import { isModerationDm } from "@/features/moderation/lib/moderationDm";
import { useUserSearchQuery } from "@/features/profile/hooks";
import { rankUserCandidatesBySearch } from "@/features/profile/lib/userCandidateSearch";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import type { Channel, UserSearchResult } from "@/shared/api/types";
import { normalizePubkey, truncateNpub } from "@/shared/lib/pubkey";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import {
  MODAL_SEARCH_INPUT_CLASS,
  MODAL_SEARCH_SHELL_CLASS,
} from "@/shared/ui/modalSearchStyles";

/** How many directory matches the people section offers at once. */
const PEOPLE_LIMIT = 10;

/**
 * The channels a message can be forwarded into, best match first.
 *
 * Excluded, in order of why:
 * - the source channel, because a copy next to its own original is noise;
 * - forum channels, because the send mutation rejects them outright, so
 *   offering one would only produce a failed send;
 * - channels the viewer has left or that are archived, because the forward
 *   would be rejected or land where nobody is reading;
 * - the moderation DM with the relay identity, which the viewer cannot write
 *   into at all (see {@link isModerationDm});
 * - huddle backing channels the shell keeps hidden, because they are call
 *   plumbing rather than a conversation anyone is reading.
 *
 * Exported for unit testing.
 */
export function selectForwardTargets({
  channels,
  currentPubkey,
  hiddenChannelIds,
  query,
  relaySelfPubkey,
  sourceChannelId,
}: {
  channels: readonly Channel[];
  currentPubkey?: string;
  /** Channels the shell hides from the sidebar (unrevealed huddle backings). */
  hiddenChannelIds?: ReadonlySet<string>;
  query: string;
  relaySelfPubkey?: string | null;
  sourceChannelId: string | null | undefined;
}): Channel[] {
  const lowerQuery = query.trim().toLowerCase();
  const source = (sourceChannelId ?? "").trim().toLowerCase();

  return channels
    .filter(
      (channel) =>
        channel.id.toLowerCase() !== source &&
        channel.channelType !== "forum" &&
        channel.isMember &&
        channel.archivedAt === null &&
        !hiddenChannelIds?.has(channel.id) &&
        !isModerationDm(channel, currentPubkey, relaySelfPubkey),
    )
    .map((channel) => ({
      channel,
      score: scoreChannelMatch(channel, lowerQuery),
    }))
    .filter(
      (entry): entry is { channel: Channel; score: number } =>
        entry.score !== null,
    )
    .sort(
      // `scoreChannelMatch` scores lower-is-better; an empty query scores
      // every channel equally, which leaves the alphabetical tiebreak.
      (a, b) =>
        a.score - b.score || a.channel.name.localeCompare(b.channel.name),
    )
    .map((entry) => entry.channel);
}

function personLabel(user: UserSearchResult): string {
  return (
    user.displayName?.trim() ||
    user.nip05Handle?.trim() ||
    truncateNpub(user.pubkey)
  );
}

/**
 * The people a message can be forwarded to, best match first.
 *
 * Someone an offered DM channel already covers is excluded, so they are picked
 * once as that conversation rather than listed twice. The relay's own identity
 * is excluded too: a DM with it is the moderation channel, which the viewer
 * cannot write into. Agents are not offered — whether a given agent accepts a
 * DM is governed by the mention and ownership rules the compose surfaces
 * apply, and this picker has no business re-deciding that.
 *
 * Exported for unit testing.
 */
export function selectForwardPeopleTargets({
  candidates,
  currentPubkey,
  excludedPubkeys,
  isArchived,
  query,
  relaySelfPubkey,
}: {
  candidates: readonly UserSearchResult[];
  currentPubkey?: string;
  /** Pubkeys already reachable through an offered DM channel. */
  excludedPubkeys?: ReadonlySet<string>;
  isArchived?: (pubkey: string) => boolean;
  query: string;
  relaySelfPubkey?: string | null;
}): UserSearchResult[] {
  const self = currentPubkey ? normalizePubkey(currentPubkey) : null;
  const relaySelf = relaySelfPubkey ? normalizePubkey(relaySelfPubkey) : null;
  const eligible = candidates.filter((candidate) => {
    const pubkey = normalizePubkey(candidate.pubkey);
    return (
      !candidate.isAgent &&
      pubkey.length > 0 &&
      pubkey !== self &&
      pubkey !== relaySelf &&
      !excludedPubkeys?.has(pubkey) &&
      !isArchived?.(pubkey)
    );
  });

  return rankUserCandidatesBySearch({
    candidates: eligible,
    getLabel: personLabel,
    limit: PEOPLE_LIMIT,
    query,
  });
}

function TargetRow({
  children,
  onSelect,
  testId,
}: {
  children: React.ReactNode;
  onSelect: () => void;
  testId: string;
}) {
  return (
    <li>
      <button
        className="flex w-full items-center gap-2 rounded-lg px-2 py-2 text-left text-sm transition-colors hover:bg-muted focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
        data-testid={testId}
        onClick={onSelect}
        type="button"
      >
        {children}
      </button>
    </li>
  );
}

function TargetSection({
  children,
  label,
}: {
  children: React.ReactNode;
  label: string;
}) {
  return (
    <section>
      <h3 className="px-2 pb-1 pt-2 text-2xs font-medium uppercase tracking-wide text-muted-foreground">
        {label}
      </h3>
      <ul className="flex flex-col gap-0.5">{children}</ul>
    </section>
  );
}

function channelLabel(channel: Channel): string {
  return channel.channelType === "dm" ? channel.name : `#${channel.name}`;
}

/**
 * Destination picker for "Forward". Choosing a channel or a person hands it to
 * the caller, which owns the send and the success/failure feedback.
 *
 * The people section only appears once something is typed: the unsearched view
 * is the viewer's own conversations, and pulling the whole relay directory to
 * fill a list nobody asked for is both noisy and a wasted round trip.
 */
export function ForwardMessageDialog({
  channels,
  currentPubkey,
  hiddenChannelIds,
  onForwardChannel,
  onForwardPerson,
  onOpenChange,
  open,
  preview,
  relaySelfPubkey,
  sourceChannelId,
}: {
  channels: readonly Channel[];
  currentPubkey?: string;
  hiddenChannelIds?: ReadonlySet<string>;
  onForwardChannel: (channel: Channel) => void;
  onForwardPerson: (person: UserSearchResult) => void;
  onOpenChange: (open: boolean) => void;
  open: boolean;
  /** One-line excerpt of the body being forwarded, for confirmation. */
  preview: string;
  relaySelfPubkey?: string | null;
  sourceChannelId: string | null | undefined;
}) {
  const [query, setQuery] = React.useState("");
  // Each open is a fresh pick; a stale query from last time would silently
  // hide most destinations.
  React.useEffect(() => {
    if (open) setQuery("");
  }, [open]);

  const trimmedQuery = query.trim();
  const deferredQuery = React.useDeferredValue(trimmedQuery);
  const isArchived = useIsArchivedPredicate(open);
  const peopleQuery = useUserSearchQuery(deferredQuery, {
    enabled: open && deferredQuery.length > 0,
    limit: PEOPLE_LIMIT,
  });

  const targets = React.useMemo(
    () =>
      selectForwardTargets({
        channels,
        currentPubkey,
        hiddenChannelIds,
        query,
        relaySelfPubkey,
        sourceChannelId,
      }),
    [
      channels,
      currentPubkey,
      hiddenChannelIds,
      query,
      relaySelfPubkey,
      sourceChannelId,
    ],
  );
  const people = React.useMemo(
    () =>
      selectForwardPeopleTargets({
        candidates: peopleQuery.data ?? [],
        currentPubkey,
        // Keyed off the *offered* channels, not every cached DM: a person
        // whose DM is archived has no channel row, so the person row is the
        // only way left to reach them.
        excludedPubkeys: oneToOneDmPubkeys(targets, currentPubkey),
        isArchived,
        query: deferredQuery,
        relaySelfPubkey,
      }),
    [
      currentPubkey,
      deferredQuery,
      isArchived,
      peopleQuery.data,
      relaySelfPubkey,
      targets,
    ],
  );
  const isEmpty = targets.length === 0 && people.length === 0;

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Forward message</DialogTitle>
          {preview ? (
            <DialogDescription className="line-clamp-2">
              {preview}
            </DialogDescription>
          ) : null}
        </DialogHeader>

        <div className={MODAL_SEARCH_SHELL_CLASS}>
          <Search aria-hidden="true" className="size-4 shrink-0" />
          <input
            aria-label="Search channels and people"
            autoFocus
            className={MODAL_SEARCH_INPUT_CLASS}
            data-testid="forward-message-search"
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Search channels and people"
            value={query}
          />
        </div>

        <div className="mt-3 max-h-80 overflow-y-auto">
          {isEmpty ? (
            <p className="px-1 py-6 text-center text-sm text-muted-foreground">
              No one to forward to.
            </p>
          ) : null}
          {targets.length > 0 ? (
            <TargetSection label="Conversations">
              {targets.map((channel) => (
                <TargetRow
                  key={channel.id}
                  onSelect={() => onForwardChannel(channel)}
                  testId={`forward-target-${channel.id}`}
                >
                  <ChannelGlyph channel={channel} />
                  <span className="min-w-0 truncate">
                    {channelLabel(channel)}
                  </span>
                </TargetRow>
              ))}
            </TargetSection>
          ) : null}
          {people.length > 0 ? (
            <TargetSection label="People">
              {people.map((person) => (
                <TargetRow
                  key={person.pubkey}
                  onSelect={() => onForwardPerson(person)}
                  testId={`forward-person-${person.pubkey}`}
                >
                  <ProfileAvatar
                    avatarUrl={person.avatarUrl}
                    className="h-6 w-6 text-2xs shadow-none"
                    iconClassName="h-3 w-3"
                    label={personLabel(person)}
                  />
                  <span className="min-w-0 truncate">
                    {personLabel(person)}
                  </span>
                </TargetRow>
              ))}
            </TargetSection>
          ) : null}
        </div>
      </DialogContent>
    </Dialog>
  );
}
