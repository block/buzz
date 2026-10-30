import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import {
  forwardedFromAttribution,
  getForwardedFromReference,
} from "@/features/messages/lib/forwardedMessage";
import type { ParsedMessageLink } from "@/features/messages/lib/messageLink";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { useChannelNavigation } from "@/shared/context/ChannelNavigationContext";
import { MessageLinkPill } from "@/shared/ui/markdown/MessageLinkPill";
import { MESSAGE_MARKDOWN_CLASS } from "@/shared/ui/mentionChip";

/**
 * Attribution for a forwarded copy: who wrote the original, and a link back
 * to it in the channel it came from.
 *
 * The source channel is read from the provenance tag rather than from the
 * row's own channel, because a forward's whole point is that those two
 * differ. The pill resolves a channel the viewer isn't in to a non-openable
 * label, so provenance stays readable without leaking a jump into a channel
 * the viewer can't read.
 *
 * An edited copy drops the author's name (see
 * {@link forwardedFromAttribution}): the forwarder can rewrite their copy, and
 * the provenance tag does not change when they do.
 */
export function ForwardedFromLine({
  currentPubkey,
  edited,
  profiles,
  tags,
}: {
  currentPubkey?: string;
  /** True when the forwarded copy itself was edited after being sent. */
  edited?: boolean;
  profiles?: UserProfileLookup;
  tags?: string[][];
}) {
  const { channels } = useChannelNavigation();
  const { goChannel } = useAppNavigation();
  const reference = getForwardedFromReference(tags);
  const onOpenMessageLink = React.useCallback(
    (target: ParsedMessageLink) => {
      void goChannel(target.channelId, {
        messageId: target.messageId,
        threadRootId: target.threadRootId,
      });
    },
    [goChannel],
  );

  if (!reference) return null;
  const link: ParsedMessageLink = {
    channelId: reference.sourceChannelId,
    messageId: reference.sourceEventId,
    threadRootId: reference.threadRootId,
  };
  const author = resolveUserLabel({
    currentPubkey,
    profiles,
    pubkey: reference.authorPubkey,
  });

  return (
    <div
      className={`${MESSAGE_MARKDOWN_CLASS} mb-1 flex min-h-[var(--inline-chip-min-height)] min-w-0 items-center gap-1.5 pt-0.5 text-sm font-normal leading-4 text-muted-foreground/70`}
      data-testid="forwarded-from"
    >
      <span className="shrink-0">
        {forwardedFromAttribution(author, edited)}
      </span>
      <MessageLinkPill
        channels={channels}
        interactive
        link={link}
        onOpenChannel={(targetChannelId) => {
          void goChannel(targetChannelId);
        }}
        onOpenMessageLink={onOpenMessageLink}
        resolveChannelReference
      />
    </div>
  );
}
