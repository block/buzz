import { Forward } from "lucide-react";
import * as React from "react";

import {
  parseForwardEnvelope,
  type ForwardSourceType,
} from "@/features/messages/lib/forwardMessage";
import { formatTime } from "@/features/messages/lib/dateFormatters";
import { useMessageEmoji } from "@/features/messages/lib/useMessageEmoji";
import { MessageTimestamp } from "@/features/messages/ui/MessageTimestamp";
import type { TimelineMessage } from "@/features/messages/types";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey, truncatePubkey } from "@/shared/lib/pubkey";
import { resolveMentionProps } from "@/shared/lib/resolveMentionNames";
import { useChannelNavigation } from "@/shared/context/ChannelNavigationContext";
import { Markdown } from "@/shared/ui/markdown";
import { parseImetaTags } from "@/shared/ui/markdown/parseImeta";
import { UserAvatar } from "@/shared/ui/UserAvatar";

/**
 * Presentational shell for a forwarded message: the forwarder's note (when
 * present), an attribution row, and the quoted original. Rendered by both the
 * timeline (via `ForwardedMessageRow`) and the forward dialog's preview so
 * what you see in the dialog is exactly what lands in the destination.
 */
export function ForwardedMessageCard({
  authorAvatarUrl,
  authorDisplayName,
  children,
  note,
  onOpenSource,
  originalCreatedAt,
  originalPubkey,
  sourceChannelName,
  sourceType,
  testId,
}: {
  /** Resolved avatar URL for the original author. */
  authorAvatarUrl?: string | null;
  /** Resolved display name for the original author; falls back to a
   *  truncated pubkey when no profile is available. */
  authorDisplayName?: string | null;
  /** Rendered markdown body of the ORIGINAL message (with its imeta map). */
  children: React.ReactNode;
  /** Rendered note node; omit entirely when the forwarder left no note. */
  note?: React.ReactNode;
  /** Jump-to-original handler; only open-channel sources are linkable. */
  onOpenSource?: () => void;
  originalCreatedAt: number;
  originalPubkey: string;
  /** Source channel name for open-channel attribution (no leading '#'). */
  sourceChannelName?: string | null;
  sourceType: ForwardSourceType;
  testId?: string;
}) {
  const displayName =
    authorDisplayName?.trim() || truncatePubkey(originalPubkey);

  const attribution =
    sourceType === "channel" ? (
      <>
        <span>Forwarded from</span>
        {onOpenSource ? (
          <button
            className="cursor-pointer truncate font-medium text-muted-foreground hover:text-foreground hover:underline focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
            data-testid="forwarded-from-channel"
            onClick={onOpenSource}
            type="button"
          >
            #{sourceChannelName ?? "channel"}
          </button>
        ) : (
          <span
            className="truncate font-medium"
            data-testid="forwarded-from-channel"
          >
            #{sourceChannelName ?? "channel"}
          </span>
        )}
      </>
    ) : (
      <span data-testid="forwarded-from-private">
        Forwarded from{" "}
        {sourceType === "dm" ? "a direct message" : "a private channel"}
      </span>
    );

  return (
    <div className="flex min-w-0 flex-col gap-1" data-testid={testId}>
      {note ? <div className="min-w-0">{note}</div> : null}
      <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
        <Forward aria-hidden className="h-3.5 w-3.5 shrink-0" />
        {attribution}
      </div>
      <div className="min-w-0 rounded-2xl border-l-2 border-border bg-muted/55 px-3 py-2">
        <div className="flex min-w-0 items-center gap-2">
          <UserAvatar
            avatarUrl={authorAvatarUrl ?? null}
            displayName={displayName}
            size="sm"
            testId="forwarded-author-avatar"
          />
          <span className="truncate text-sm font-semibold leading-4">
            {displayName}
          </span>
          <MessageTimestamp
            createdAt={originalCreatedAt}
            time={formatTime(originalCreatedAt)}
          />
        </div>
        <div className="mt-1 min-w-0">{children}</div>
      </div>
    </div>
  );
}

/**
 * Timeline body for a kind-40009 event: parses the forward envelope from the
 * event's tags and renders the shared `ForwardedMessageCard`. The embedded
 * original's author and mentions resolve out of the surface's `profiles`
 * lookup (see `collectForwardEmbeddedPubkeys`), never a per-row fetch.
 * Open-channel attributions link back to the original through the same
 * channel+messageId route message links use.
 */
export function ForwardedMessageRow({
  message,
  profiles,
}: {
  message: TimelineMessage;
  profiles?: UserProfileLookup;
}) {
  const envelope = React.useMemo(
    () => parseForwardEnvelope(message.tags ?? []),
    [message.tags],
  );
  const original = envelope?.original ?? null;
  const originalPubkey = original ? normalizePubkey(original.pubkey) : "";
  const { channels, nonDmChannelNames } = useChannelNavigation();
  const { goChannel } = useAppNavigation();

  // No per-row profile query: the embedded author and the embedded original's
  // mentions are collected into the surface's batched request by
  // `collectForwardEmbeddedPubkeys`.
  const authorProfile = originalPubkey ? profiles?.[originalPubkey] : undefined;

  const noteMentions = React.useMemo(
    () => resolveMentionProps(message.tags, profiles),
    [message.tags, profiles],
  );
  const originalMentions = React.useMemo(
    () => resolveMentionProps(original?.tags, profiles),
    [original?.tags, profiles],
  );
  const originalImetaByUrl = React.useMemo(
    () => (original ? parseImetaTags(original.tags) : undefined),
    [original],
  );
  const { customEmoji: originalCustomEmoji } = useMessageEmoji(
    original?.content ?? "",
    original?.tags,
  );

  const sourceChannelId = envelope?.sourceChannelId;
  const originalId = original?.id;
  const handleOpenSource = React.useCallback(() => {
    if (!sourceChannelId || !originalId) return;
    void goChannel(sourceChannelId, { messageId: originalId });
  }, [goChannel, originalId, sourceChannelId]);

  if (!envelope || !original) {
    // The relay validates forwards on ingest, so this only occurs for
    // malformed events from before validation (or other clients' bugs).
    return (
      <p className="text-sm italic text-muted-foreground">
        This forwarded message can't be displayed.
      </p>
    );
  }

  const sourceChannelName =
    envelope.sourceType === "channel"
      ? (channels.find((channel) => channel.id === envelope.sourceChannelId)
          ?.name ?? null)
      : null;

  return (
    <ForwardedMessageCard
      authorAvatarUrl={authorProfile?.avatarUrl ?? null}
      authorDisplayName={authorProfile?.displayName ?? null}
      note={
        message.body.trim().length > 0 ? (
          <Markdown
            channelNames={nonDmChannelNames}
            className="max-w-full text-sm"
            content={message.body}
            mentionNames={noteMentions.mentionNames}
            mentionPubkeysByName={noteMentions.mentionPubkeysByName}
          />
        ) : undefined
      }
      onOpenSource={
        envelope.sourceType === "channel" ? handleOpenSource : undefined
      }
      originalCreatedAt={original.created_at}
      originalPubkey={originalPubkey}
      sourceChannelName={sourceChannelName}
      sourceType={envelope.sourceType}
      testId={`forwarded-message-${message.id}`}
    >
      <Markdown
        channelNames={nonDmChannelNames}
        className={cn("max-w-full text-sm")}
        content={original.content}
        customEmoji={originalCustomEmoji}
        imetaByUrl={originalImetaByUrl}
        mentionNames={originalMentions.mentionNames}
        mentionPubkeysByName={originalMentions.mentionPubkeysByName}
      />
    </ForwardedMessageCard>
  );
}
