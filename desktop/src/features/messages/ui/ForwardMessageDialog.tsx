import { Check, Hash, Link2, MessageCircle, Search } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import {
  useManagedAgentsQuery,
  useRelayAgentsQuery,
} from "@/features/agents/hooks";
import {
  useChannelMembersQuery,
  useChannelsQuery,
} from "@/features/channels/hooks";
import { useMessageProfiles } from "@/features/channels/ui/useMessageProfiles";
import {
  forwardSourceTypeForChannel,
  parseForwardEnvelope,
  type ForwardSourceType,
} from "@/features/messages/lib/forwardMessage";
import {
  noteMayMention,
  resolveForwardNoteMentions,
} from "@/features/messages/lib/forwardNoteMentions";
import { buildMessageLink } from "@/features/messages/lib/messageLink";
import { getThreadReference } from "@/features/messages/lib/threading";
import { useMessageEmoji } from "@/features/messages/lib/useMessageEmoji";
import { useForwardMessageMutation } from "@/features/messages/hooks";
import { ForwardedMessageCard } from "@/features/messages/ui/ForwardedMessageCard";
import { scoreChannelMatch } from "@/features/channels/lib/channelSearchScore";
import { useProfileQuery, useUsersBatchQuery } from "@/features/profile/hooks";
import { resolveChannelDisplayLabel } from "@/features/sidebar/lib/channelLabels";
import type { Channel } from "@/shared/api/types";
import { KIND_STREAM_MESSAGE_FORWARD } from "@/shared/constants/kinds";
import { copyTextToClipboard } from "@/shared/lib/clipboard";
import { cn } from "@/shared/lib/cn";
import { useChannelNavigation } from "@/shared/context/ChannelNavigationContext";
import { normalizePubkey } from "@/shared/lib/pubkey";
import {
  getMentionTagPubkey,
  resolveMentionProps,
} from "@/shared/lib/resolveMentionNames";
import { Button } from "@/shared/ui/button";
import { ChooserDialogContent } from "@/shared/ui/chooser-dialog-content";
import { Dialog } from "@/shared/ui/dialog";
import { Markdown } from "@/shared/ui/markdown";
import { parseImetaTags } from "@/shared/ui/markdown/parseImeta";
import {
  MODAL_SEARCH_INPUT_CLASS,
  MODAL_SEARCH_SHELL_CLASS,
} from "@/shared/ui/modalSearchStyles";
import { Textarea } from "@/shared/ui/textarea";

import type { ForwardMessageTarget } from "./ForwardMessageProvider";

/**
 * How long sending waits for the destination's member list before forwarding
 * without mention tags — long enough for a normal round trip, short enough that
 * a request that never settles doesn't strand the Forward button.
 */
const MEMBER_WAIT_LIMIT_MS = 4_000;

function channelActivityTime(channel: Channel) {
  if (!channel.lastMessageAt) return 0;
  const timestamp = Date.parse(channel.lastMessageAt);
  return Number.isFinite(timestamp) ? timestamp : 0;
}

type DestinationRow = {
  channel: Channel;
  label: string;
};

/**
 * "Forward message…" dialog: searchable destination picker over channels AND
 * DMs (DMs are ordinary channels with `channel_type='dm'`), an optional note,
 * and a WYSIWYG preview of the original rendered with the same
 * `ForwardedMessageCard` the destination timeline uses.
 */
export function ForwardMessageDialog({
  currentPubkey,
  onOpenChange,
  open,
  target,
}: {
  currentPubkey?: string;
  onOpenChange: (open: boolean) => void;
  open: boolean;
  target: ForwardMessageTarget | null;
}) {
  const [query, setQuery] = React.useState("");
  const [note, setNote] = React.useState("");
  const [selectedId, setSelectedId] = React.useState<string | null>(null);
  const [highlightIndex, setHighlightIndex] = React.useState<number | null>(
    null,
  );
  /** Destination whose member list took too long to load (see MEMBER_WAIT_LIMIT_MS). */
  const [waitExpiredForId, setWaitExpiredForId] = React.useState<string | null>(
    null,
  );

  const message = target?.message ?? null;
  const forwardMutation = useForwardMessageMutation(currentPubkey);
  const { nonDmChannelNames } = useChannelNavigation();
  const channelsQuery = useChannelsQuery({ enabled: open });
  const channels = React.useMemo(
    () => channelsQuery.data ?? [],
    [channelsQuery.data],
  );

  React.useEffect(() => {
    if (!open) {
      setQuery("");
      setNote("");
      setSelectedId(null);
      setHighlightIndex(null);
      setWaitExpiredForId(null);
    }
  }, [open]);

  // Destinations: joinable message surfaces only — no forums, no archived
  // channels; DMs are always available to their participants.
  const candidates = React.useMemo(
    () =>
      channels.filter(
        (channel) =>
          !channel.archivedAt &&
          channel.channelType !== "forum" &&
          (channel.isMember || channel.channelType === "dm"),
      ),
    [channels],
  );

  const dmParticipantPubkeys = React.useMemo(
    () =>
      candidates
        .filter((channel) => channel.channelType === "dm")
        .flatMap((channel) => channel.participantPubkeys),
    [candidates],
  );
  const dmProfilesQuery = useUsersBatchQuery(dmParticipantPubkeys, {
    enabled: open && dmParticipantPubkeys.length > 0,
  });

  const rows = React.useMemo<DestinationRow[]>(
    () =>
      candidates.map((channel) => ({
        channel,
        label:
          channel.channelType === "dm"
            ? resolveChannelDisplayLabel(
                channel,
                currentPubkey,
                dmProfilesQuery.data?.profiles,
              )
            : channel.name,
      })),
    [candidates, currentPubkey, dmProfilesQuery.data],
  );

  const normalizedQuery = query.trim().toLowerCase();
  const orderedRows = React.useMemo(() => {
    const scores = new Map<string, number>();
    if (normalizedQuery.length > 0) {
      for (const row of rows) {
        const score = scoreChannelMatch(
          { name: row.label, description: row.channel.description },
          normalizedQuery,
        );
        if (score !== null) scores.set(row.channel.id, score);
      }
    }

    const visible =
      normalizedQuery.length === 0
        ? [...rows]
        : rows.filter((row) => scores.has(row.channel.id));

    return visible.sort((a, b) => {
      if (normalizedQuery.length > 0) {
        const scoreDiff =
          (scores.get(a.channel.id) ?? Number.POSITIVE_INFINITY) -
          (scores.get(b.channel.id) ?? Number.POSITIVE_INFINITY);
        if (scoreDiff !== 0) return scoreDiff;
      }
      const activityDiff =
        channelActivityTime(b.channel) - channelActivityTime(a.channel);
      if (activityDiff !== 0) return activityDiff;
      return a.label.localeCompare(b.label, undefined, {
        sensitivity: "base",
      });
    });
  }, [normalizedQuery, rows]);

  const selectedRow =
    orderedRows.find((row) => row.channel.id === selectedId) ??
    rows.find((row) => row.channel.id === selectedId) ??
    null;

  // Note @mentions resolve against the DESTINATION's members — the note is
  // published into that channel, so its members are the mentionable set.
  const destinationId = selectedRow?.channel.id ?? null;
  const destinationMembersQuery = useChannelMembersQuery(destinationId, open);
  const noteMentions = React.useMemo(
    () => resolveForwardNoteMentions(note, destinationMembersQuery.data),
    [note, destinationMembersQuery.data],
  );
  // The note's `p` tags are derived from those members, so submitting before
  // they arrive would silently publish a mention that notifies nobody. Block
  // only while the query is genuinely in flight — if it fails, the forward
  // still goes out, just without mention tags.
  const membersInFlight =
    noteMayMention(note) && destinationMembersQuery.isPending;
  // …and bound that wait the same way: a request that never settles would
  // otherwise disable Forward forever for any note the preflight matches (an
  // email address is enough), so past the deadline sending proceeds without
  // mention tags exactly as it does on the error path. The deadline is recorded
  // per destination, so picking another one waits for its members afresh.
  React.useEffect(() => {
    if (!membersInFlight || destinationId === null) return;
    const timer = window.setTimeout(() => {
      setWaitExpiredForId(destinationId);
    }, MEMBER_WAIT_LIMIT_MS);
    return () => {
      window.clearTimeout(timer);
    };
  }, [destinationId, membersInFlight]);
  const mentionsUnresolved =
    membersInFlight && waitExpiredForId !== destinationId;
  // Once the wait expires (or the query fails outright) sending is allowed but
  // must not be silent — a merely slow member lookup would otherwise let a
  // mention-bearing note publish without notifying anyone while the button
  // still reads "Forward". The submit action says so explicitly, and reverts
  // to a normal Forward if member data lands before the click (handleForward
  // resolves from the data as of the click). An error only counts when there
  // is no data at all: a failed background refetch keeps the last successful
  // members, and handleForward WILL resolve mentions from them — the label
  // must not promise otherwise.
  const mentionsUnavailable =
    noteMayMention(note) &&
    destinationMembersQuery.data === undefined &&
    (destinationMembersQuery.isError ||
      (destinationMembersQuery.isPending &&
        destinationId !== null &&
        waitExpiredForId === destinationId));

  // ── Preview (WYSIWYG: exactly what the destination will render) ──────────
  // Forwarding a forward flattens: the preview shows ITS embedded original.
  const previewEnvelope = React.useMemo(
    () =>
      message?.kind === KIND_STREAM_MESSAGE_FORWARD
        ? parseForwardEnvelope(message.tags ?? [])
        : null,
    [message],
  );
  const previewSourceChannelId =
    previewEnvelope?.sourceChannelId ?? target?.channelId ?? null;
  const sourceChannel =
    channels.find((channel) => channel.id === previewSourceChannelId) ?? null;
  const sourceType: ForwardSourceType = sourceChannel
    ? forwardSourceTypeForChannel(sourceChannel)
    : (previewEnvelope?.sourceType ?? "private");

  const previewPubkey = normalizePubkey(
    previewEnvelope?.original.pubkey ??
      message?.pubkey ??
      message?.signerPubkey ??
      "",
  );
  const previewCreatedAt =
    previewEnvelope?.original.created_at ?? message?.createdAt ?? 0;
  const previewContent = previewEnvelope?.original.content ?? message?.body;
  const previewTags = previewEnvelope?.original.tags ?? message?.tags;

  // Profiles the preview needs, in one batched request: everyone the original
  // mentions — so its @mentions render as the same chips the destination
  // timeline will show — plus a flattened original's author, whose profile the
  // timeline row already resolved for non-forward messages.
  const previewMentionPubkeys = React.useMemo(
    () =>
      (previewTags ?? [])
        .map((tag) => getMentionTagPubkey(tag))
        .filter((pubkey): pubkey is string => pubkey !== null),
    [previewTags],
  );
  const previewProfilePubkeys = React.useMemo(
    () =>
      previewEnvelope && previewPubkey
        ? [previewPubkey, ...previewMentionPubkeys]
        : previewMentionPubkeys,
    [previewEnvelope, previewMentionPubkeys, previewPubkey],
  );
  const previewProfilesQuery = useUsersBatchQuery(previewProfilePubkeys, {
    enabled: open,
  });
  // The batch alone is not what the destination timeline renders from: it
  // overlays the current profile and managed/relay agent names on top
  // (`useMessageProfiles`), which is the only name source for an agent with no
  // kind:0 profile. Reusing that hook keeps a preview mention of such an agent
  // a named chip instead of plain text. Channel members are omitted — they only
  // contribute `isAgent` flags, which neither mention chips nor the author line
  // read.
  const currentProfileQuery = useProfileQuery(open);
  const managedAgentsQuery = useManagedAgentsQuery({ enabled: open });
  const relayAgentsQuery = useRelayAgentsQuery({ enabled: open });
  const previewProfiles = useMessageProfiles({
    channelMembers: undefined,
    currentProfile: currentProfileQuery.data,
    currentPubkey,
    managedAgents: managedAgentsQuery.data ?? [],
    profiles: previewProfilesQuery.data?.profiles,
    relayAgents: relayAgentsQuery.data ?? [],
  });
  const flattenAuthor = previewProfiles[previewPubkey];
  const previewAuthorName = previewEnvelope
    ? (flattenAuthor?.displayName ?? null)
    : (message?.author ?? null);
  const previewAuthorAvatarUrl = previewEnvelope
    ? (flattenAuthor?.avatarUrl ?? null)
    : (message?.avatarUrl ?? null);

  const previewImetaByUrl = React.useMemo(
    () => (previewTags ? parseImetaTags(previewTags) : undefined),
    [previewTags],
  );
  const previewMentions = React.useMemo(
    () => resolveMentionProps(previewTags, previewProfiles),
    [previewTags, previewProfiles],
  );
  const { customEmoji: previewCustomEmoji } = useMessageEmoji(
    previewContent ?? "",
    previewTags,
  );

  const handleForward = () => {
    if (!message || !selectedRow || forwardMutation.isPending) return;
    if (mentionsUnresolved) return;
    const destinationLabel =
      selectedRow.channel.channelType === "dm"
        ? selectedRow.label
        : `#${selectedRow.label}`;
    // Resolved here rather than reused from the preview memo so the recipients
    // come from the members data as of the click.
    const mentions = resolveForwardNoteMentions(
      note,
      destinationMembersQuery.data,
    );
    forwardMutation.mutate(
      {
        destination: selectedRow.channel,
        note,
        mentionPubkeys: mentions.pubkeys,
        message,
      },
      {
        onSuccess: () => {
          toast.success(`Message forwarded to ${destinationLabel}`);
          onOpenChange(false);
        },
        onError: (error) => {
          toast.error(`Failed to forward message: ${error.message}`);
        },
      },
    );
  };

  const handleCopyLink = () => {
    if (!message || !target) return;
    const { rootId } = getThreadReference(message.tags ?? []);
    const link = buildMessageLink({
      channelId: target.channelId,
      messageId: message.id,
      threadRootId: rootId,
    });
    copyTextToClipboard(link, "Link copied to clipboard");
  };

  const handleSearchKeyDown = (
    event: React.KeyboardEvent<HTMLInputElement>,
  ) => {
    if (event.key === "ArrowDown" && orderedRows.length > 0) {
      event.preventDefault();
      setHighlightIndex((current) =>
        current === null ? 0 : Math.min(current + 1, orderedRows.length - 1),
      );
      return;
    }

    if (event.key === "ArrowUp" && orderedRows.length > 0) {
      event.preventDefault();
      setHighlightIndex((current) =>
        current === null ? orderedRows.length - 1 : Math.max(current - 1, 0),
      );
      return;
    }

    if (
      event.key === "Enter" &&
      !event.nativeEvent.isComposing &&
      orderedRows.length > 0
    ) {
      event.preventDefault();
      const row = orderedRows[highlightIndex ?? 0];
      if (row) setSelectedId(row.channel.id);
    }
  };

  if (!target || !message) {
    return null;
  }

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <ChooserDialogContent
        aria-describedby={undefined}
        data-testid="forward-message-dialog"
        footer={
          <>
            <Button onClick={handleCopyLink} type="button" variant="outline">
              <Link2 className="h-4 w-4" />
              Copy link
            </Button>
            {mentionsUnavailable && (
              <span className="ml-auto text-xs text-muted-foreground">
                Member list unavailable — @mentions won't notify
              </span>
            )}
            <Button
              className={mentionsUnavailable ? undefined : "ml-auto"}
              data-testid="forward-message-submit"
              disabled={
                !selectedRow || forwardMutation.isPending || mentionsUnresolved
              }
              onClick={handleForward}
              type="button"
            >
              {forwardMutation.isPending
                ? "Forwarding…"
                : mentionsUnavailable
                  ? "Forward without mentions"
                  : "Forward"}
            </Button>
          </>
        }
        footerClassName="items-center gap-3"
        title="Forward message"
      >
        <div className="flex flex-col gap-4">
          <div className={cn(MODAL_SEARCH_SHELL_CLASS, "mt-0")}>
            <label
              className="flex min-w-0 flex-1 cursor-text items-center gap-3"
              htmlFor="forward-destination-search"
            >
              <Search className="h-4 w-4 shrink-0 text-muted-foreground/55 transition-colors duration-150 ease-out group-hover/search:text-muted-foreground group-focus-within/search:text-foreground" />
              <input
                autoCapitalize="none"
                autoCorrect="off"
                className={MODAL_SEARCH_INPUT_CLASS}
                data-testid="forward-destination-search"
                id="forward-destination-search"
                onChange={(event) => {
                  setQuery(event.target.value);
                  setHighlightIndex(null);
                }}
                onKeyDown={handleSearchKeyDown}
                placeholder="Search for a channel or person"
                spellCheck={false}
                type="text"
                value={query}
              />
            </label>
          </div>

          {orderedRows.length === 0 ? (
            <p className="px-1 py-3 text-sm text-muted-foreground">
              No destinations match your search.
            </p>
          ) : (
            <div className="max-h-56 overflow-y-auto rounded-xl border border-border/70 bg-background/70 shadow-xs divide-y divide-border/55">
              {orderedRows.map(({ channel, label }, index) => {
                const isSelected = channel.id === selectedId;
                const isHighlighted = index === highlightIndex;
                const Icon =
                  channel.channelType === "dm" ? MessageCircle : Hash;
                return (
                  <button
                    className={cn(
                      "flex w-full items-center gap-3 px-3 py-2.5 text-left transition-colors duration-150 ease-out focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring",
                      isSelected
                        ? "bg-primary/10"
                        : isHighlighted
                          ? "bg-muted/60"
                          : "hover:bg-muted/40",
                    )}
                    data-testid={`forward-destination-${channel.id}`}
                    key={channel.id}
                    onClick={() => setSelectedId(channel.id)}
                    onMouseEnter={() => setHighlightIndex(index)}
                    type="button"
                  >
                    <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-secondary text-secondary-foreground">
                      <Icon className="h-4 w-4" />
                    </span>
                    <span className="min-w-0 flex-1 truncate text-sm font-medium">
                      {label}
                    </span>
                    {isSelected ? (
                      <Check className="h-4 w-4 shrink-0 text-primary" />
                    ) : null}
                  </button>
                );
              })}
            </div>
          )}

          <Textarea
            data-testid="forward-message-note"
            onChange={(event) => setNote(event.target.value)}
            placeholder="Add a message, if you'd like"
            value={note}
          />

          {sourceType !== "channel" ? (
            <p
              className="text-xs text-muted-foreground"
              data-testid="forward-privacy-notice"
            >
              Forwarding shares this{" "}
              {sourceType === "dm" ? "direct message" : "private channel"}{" "}
              content with everyone in the destination.
            </p>
          ) : null}

          <div className="rounded-2xl border border-border/60 px-3 py-2.5">
            <ForwardedMessageCard
              authorAvatarUrl={previewAuthorAvatarUrl}
              authorDisplayName={previewAuthorName}
              note={
                note.trim().length > 0 ? (
                  <Markdown
                    channelNames={nonDmChannelNames}
                    className="max-w-full text-sm"
                    content={note}
                    mentionNames={
                      noteMentions.names.length > 0
                        ? noteMentions.names
                        : undefined
                    }
                    mentionPubkeysByName={
                      noteMentions.names.length > 0
                        ? noteMentions.pubkeysByName
                        : undefined
                    }
                  />
                ) : undefined
              }
              originalCreatedAt={previewCreatedAt}
              originalPubkey={previewPubkey}
              sourceChannelName={
                sourceType === "channel" ? (sourceChannel?.name ?? null) : null
              }
              sourceType={sourceType}
              testId="forward-message-preview"
            >
              <Markdown
                channelNames={nonDmChannelNames}
                className="max-w-full text-sm"
                content={previewContent ?? ""}
                customEmoji={previewCustomEmoji}
                imetaByUrl={previewImetaByUrl}
                mentionNames={previewMentions.mentionNames}
                mentionPubkeysByName={previewMentions.mentionPubkeysByName}
              />
            </ForwardedMessageCard>
          </div>
        </div>
      </ChooserDialogContent>
    </Dialog>
  );
}
