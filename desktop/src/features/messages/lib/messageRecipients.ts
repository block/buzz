import type { Channel } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { MENTION_TAG_CAP } from "@/features/messages/lib/threading";

/** Who an outgoing message `p`-tags, split by why it tags them. */
export interface MessageRecipients {
  /** Typed as `@name` in the body. Marked `mention` on a reply. */
  mentions: string[];
  /**
   * Addressed by the *channel* rather than by the message — every other
   * participant in a DM, tagged whether or not anyone typed their name.
   * Never marked: neither role is true of it.
   */
  addressed: string[];
}

/**
 * Return the semantic recipients for an outgoing message, split by role.
 *
 * Stream messages notify only explicit mentions. A DM addresses every other
 * participant, so it must carry recipient `p` tags even when the composer text
 * contains no `@mention`. Agent harnesses and human notification subscriptions
 * both rely on those tags.
 *
 * The two groups stay apart because a reply marks its `p` tags with the role
 * each one plays. Folding DM participants in with typed mentions made every DM
 * thread reply claim its counterpart had been `@`-mentioned — which pierces a
 * mute and takes a slot in the mention feed ahead of a real `@you`.
 *
 * A thread reply also addresses the author it replies to, per NIP-10, but that
 * tag is **not** added here. The backend adds it, marked, from the parent event
 * `resolve_thread_ref` already fetches — which is both more reliable (this
 * function only ever saw a *cached* parent author, and the cache misses for any
 * channel not opened this session) and unambiguous on the wire. All
 * `parentAuthorPubkey` does here is reserve the tag's slot against the cap.
 */
export function messageRecipients(
  channel: Channel,
  senderPubkey: string,
  explicitMentions: readonly string[] = [],
  parentAuthorPubkey?: string | null,
): MessageRecipients {
  const sender = normalizePubkey(senderPubkey);
  const keep = (pubkey: string) => pubkey.length > 0 && pubkey !== sender;

  const mentions = [...new Set(explicitMentions.map(normalizePubkey))].filter(
    keep,
  );

  const addressed =
    channel.channelType === "dm"
      ? [
          ...new Set(
            [...channel.memberPubkeys, ...channel.participantPubkeys].map(
              normalizePubkey,
            ),
          ),
        ].filter((pubkey) => keep(pubkey) && !mentions.includes(pubkey))
      : [];

  // Past the cap the builder rejects the whole event rather than trimming, so
  // an over-full list is a failed send. The addressing tag the backend appends
  // counts toward that cap, so leave it a slot — an agent's `require_mention`
  // subscription never sees an untagged reply, while a dropped body mention
  // only costs one notification.
  const parent = normalizePubkey(parentAuthorPubkey ?? "");
  const reservesAddressingSlot =
    keep(parent) && !mentions.includes(parent) && !addressed.includes(parent);
  const cap = reservesAddressingSlot ? MENTION_TAG_CAP - 1 : MENTION_TAG_CAP;

  // Mentions keep their places ahead of channel recipients, as they did when
  // the two were one list sliced from the front.
  return {
    mentions: mentions.slice(0, cap),
    addressed: addressed.slice(0, Math.max(0, cap - mentions.length)),
  };
}
