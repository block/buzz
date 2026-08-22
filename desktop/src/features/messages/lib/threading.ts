import type { RelayEvent } from "@/shared/api/types";

export type ThreadReference = {
  parentId: string | null;
  rootId: string | null;
};

function getEventTags(tags: string[][]) {
  return tags.filter((tag) => tag[0] === "e" && typeof tag[1] === "string");
}

const EVENT_ID_HEX_RE = /^[0-9a-f]{64}$/;

/**
 * An `e` tag value if it can identify a relay event, else `null`.
 *
 * **Every caller that puts an event id into a REQ filter must go through this.**
 * The relay *stores* whatever an `e` tag contains — its NIP-10 resolver ignores a
 * malformed value instead of rejecting the event — so any community member can
 * publish one. `nostr::Filter` parses `ids` into event ids and the relay answers
 * a bare `NOTICE` when that fails: no `CLOSED`, no `EOSE`. The client ignores
 * notices that are not `rate-limited:`, so the request hangs the full history
 * timeout and then rejects. One such event in a channel is enough to make every
 * parent lookup there time out for the rest of the session, and it does not clear
 * on restart.
 *
 * `getThreadReference` deliberately does not apply this — a value that cannot
 * identify an event is still a usable thread-grouping key.
 */
export function normalizeEventId(value: string | null | undefined) {
  if (typeof value !== "string") {
    return null;
  }
  const lower = value.toLowerCase();
  return EVENT_ID_HEX_RE.test(lower) ? lower : null;
}

/**
 * Marker on a `p` tag that names the author this reply answers.
 *
 * NIP-10 addressing and a typed `@mention` produce byte-identical `p` tags, so
 * a receiver has to fetch the parent message and check who wrote it to tell them
 * apart — a relay round trip to recover something the sender knew for free.
 * These markers record it instead, in the fourth position, exactly as `e` tags
 * already carry `root` and `reply`.
 *
 * Relay tag filters match only the tag's second element
 * (`crates/buzz-core/src/filter.rs`), so a marker cannot affect `#p` delivery —
 * an agent's `require_mention` subscription still receives the reply.
 */
export const P_TAG_ADDRESSING_MARKER = "reply";

/**
 * Marker on a `p` tag that names someone the author typed as `@name`.
 *
 * Distinct from the `["mention", pk]` *reference* tag in
 * `shared/lib/resolveMentionNames.ts`, which is a different tag kind meaning
 * "render the chip but do not notify". This is a marker on a real `p` tag and
 * does notify.
 *
 * Emitted alongside {@link P_TAG_ADDRESSING_MARKER} rather than instead of it.
 * Marking only the addressing tag would leave the one case inference cannot
 * reach still unreachable: when the recipient is both the parent's author and
 * typed in the body, a single tag can only be marked or unmarked. With both
 * markers the sender simply emits this one, and mention wins.
 */
export const P_TAG_MENTION_MARKER = "mention";

export type PTagRole = "addressing" | "mention" | "unknown" | "none";

/**
 * What the `p` tags naming `pubkey` say this event is to them.
 *
 * `unknown` is load-bearing and must never be collapsed into either answer. A
 * sender that predates these markers emits a bare `p` tag for both roles, so
 * absence of a marker means "ask the parent", not "this is a mention". Only a
 * marker that is actually present is authoritative.
 */
export function pTagRoleFor(tags: string[][], pubkey: string): PTagRole {
  const target = pubkey.toLowerCase();
  let sawAddressing = false;
  let sawBare = false;
  for (const tag of tags) {
    if (tag[0] !== "p" || tag[1]?.toLowerCase() !== target) {
      continue;
    }
    // Mention wins outright: it is the only marker a sender emits when the
    // recipient is both the parent's author and typed in the body.
    if (tag[3] === P_TAG_MENTION_MARKER) {
      return "mention";
    }
    if (tag[3] === P_TAG_ADDRESSING_MARKER) {
      sawAddressing = true;
    } else {
      sawBare = true;
    }
  }
  if (sawBare) return "unknown";
  return sawAddressing ? "addressing" : "none";
}

export function getChannelIdFromTags(tags: string[][]) {
  return tags.find((tag) => tag[0] === "h")?.[1] ?? null;
}

export function isBroadcastReply(tags: string[][]): boolean {
  return tags.some((tag) => tag[0] === "broadcast" && tag[1] === "1");
}

export function isThreadReply(tags: string[][]): boolean {
  const ref = getThreadReference(tags);
  return ref.parentId !== null && !isBroadcastReply(tags);
}

export function getThreadReference(tags: string[][]): ThreadReference {
  const eventTags = getEventTags(tags);

  if (eventTags.length === 0) {
    return {
      parentId: null,
      rootId: null,
    };
  }

  const rootTag = eventTags.find((tag) => tag[3] === "root");
  const replyTag =
    [...eventTags].reverse().find((tag) => tag[3] === "reply") ?? null;

  if (!replyTag) {
    return {
      parentId: null,
      rootId: null,
    };
  }

  // Lowercased, not validated. Case has to be normalized here because these ids
  // are compared against `event.id`, which is always lowercase, and a mismatch
  // reads as "parent absent" — which relabels a reply a real mention.
  //
  // Validation deliberately does NOT happen here: this is the general
  // thread-grouping primitive, and a value that cannot identify a relay event is
  // still a usable grouping key. Callers that put the id into a REQ filter must
  // run it through `normalizeEventId` themselves.
  const parentId = replyTag[1]?.toLowerCase() ?? null;

  return {
    parentId,
    rootId: rootTag?.[1]?.toLowerCase() ?? parentId,
  };
}

/**
 * Best-effort client-side normalization of mention pubkeys: lowercase, deduplicate, skip self.
 * The relay performs authoritative validation (hex format, 64-char length, cap of 50)
 * on top of the same normalization — this helper keeps optimistic UI tags consistent.
 */
export function normalizeMentionPubkeys(
  mentionPubkeys: string[],
  selfPubkey: string,
): string[] {
  const selfLower = selfPubkey.toLowerCase();
  const seen = new Set<string>([selfLower]);
  const result: string[] = [];
  for (const pk of mentionPubkeys) {
    const lower = pk.toLowerCase();
    if (seen.has(lower)) {
      continue;
    }
    seen.add(lower);
    result.push(lower);
  }
  return result;
}

/**
 * Mentions an edit *newly adds*, relative to the original message body.
 *
 * The composer resolves both bodies to pubkey lists with the same
 * channel-roster resolver the send path uses, then hands them here. We
 * normalize the edited body's set (lowercase / dedup / drop self) and keep
 * only pubkeys that were not already present in the original body — compared
 * case-insensitively so a case-only difference is never treated as "new".
 *
 * A typo-fix edit that leaves the mention set unchanged yields `[]`, so the
 * edit event carries no `p` tags and re-wakes nobody. Only genuinely new
 * mentions get notified.
 */
export function diffAddedMentionPubkeys(
  originalPubkeys: string[],
  editedPubkeys: string[],
  selfPubkey: string,
): string[] {
  const original = new Set(originalPubkeys.map((pk) => pk.toLowerCase()));
  return normalizeMentionPubkeys(editedPubkeys, selfPubkey).filter(
    (pubkey) => !original.has(pubkey),
  );
}

export function buildReplyTags(
  channelId: string,
  authorPubkey: string,
  parentEventId: string,
  rootEventId: string,
  mentionPubkeys: string[] = [],
  parentAuthorPubkey?: string | null,
  addressedPubkeys: string[] = [],
) {
  const tags: string[][] = [
    ["p", authorPubkey],
    ["h", channelId],
  ];

  const parentAuthor = parentAuthorPubkey?.trim().toLowerCase() ?? "";
  const mentions = normalizeMentionPubkeys(mentionPubkeys, authorPubkey);
  const addressing =
    parentAuthor &&
    parentAuthor !== authorPubkey.toLowerCase() &&
    !mentions.includes(parentAuthor)
      ? parentAuthor
      : "";

  // Add p-tags for mentioned users so mention-filtered subscriptions
  // (e.g. ACP agent harness) receive the reply event.
  // Best-effort normalization — relay performs authoritative validation.
  //
  // Marked so the recipient does not have to fetch the parent to learn which
  // role each tag plays. A pubkey that is both typed and the parent's author
  // gets the mention marker only, and is not repeated as an addressing tag.
  // Order matches the backend builder so this optimistic copy is tag-identical
  // to the event the relay stores.
  for (const pubkey of mentions) {
    tags.push(["p", pubkey, "", P_TAG_MENTION_MARKER]);
  }
  // Recipients the channel addresses rather than the message — DM participants.
  // Left bare: neither marker is true of them, and under the one-way read a bare
  // tag means "ask the parent", which is the answer they had before markers.
  for (const pubkey of normalizeMentionPubkeys(
    addressedPubkeys,
    authorPubkey,
  )) {
    if (!mentions.includes(pubkey) && pubkey !== addressing) {
      tags.push(["p", pubkey]);
    }
  }
  if (addressing) {
    tags.push(["p", addressing, "", P_TAG_ADDRESSING_MARKER]);
  }

  if (parentEventId === rootEventId) {
    tags.push(["e", rootEventId, "", "reply"]);
    return tags;
  }

  tags.push(["e", rootEventId, "", "root"]);
  tags.push(["e", parentEventId, "", "reply"]);
  return tags;
}

export function buildThreadReferenceTags(
  channelId: string,
  parentEventId: string | null,
  rootEventId: string | null,
) {
  const tags: string[][] = [["h", channelId]];

  if (!parentEventId) {
    return tags;
  }

  if (!rootEventId || parentEventId === rootEventId) {
    tags.push(["e", parentEventId, "", "reply"]);
    return tags;
  }

  tags.push(["e", rootEventId, "", "root"]);
  tags.push(["e", parentEventId, "", "reply"]);
  return tags;
}

export function resolveReplyRootId(
  parentEventId: string,
  events: RelayEvent[],
) {
  const parent = events.find((event) => event.id === parentEventId);
  if (!parent) {
    return parentEventId;
  }

  const thread = getThreadReference(parent.tags);
  return thread.rootId ?? parent.id;
}

/**
 * Hard cap the relay applies to `p` tags on one event.
 *
 * Mirrors `MENTION_CAP` in `crates/buzz-sdk/src/mentions.rs`.
 */
export const MENTION_TAG_CAP = 50;

/**
 * Typed-mention `p` tags for a reply, with room reserved for the addressing tag.
 *
 * The addressing tag itself is **not** in this list. The backend appends it,
 * marked, from the parent event `resolve_thread_ref` already fetches — which is
 * both unambiguous on the wire and more reliable than any caller here, since
 * every caller sourced the parent author from a cache that misses for channels
 * the user has not opened. A miss used to mean the reply shipped with no
 * addressing tag at all, and an agent's `require_mention` subscription never
 * received it.
 *
 * What survives is the reservation. The builder rejects the *whole* event past
 * the cap rather than trimming, so a reply composed with 50 mentions plus the
 * addressing tag would simply fail to send. Dropping a body mention costs one
 * notification; dropping the addressing tag costs the agent the reply.
 */
export function replyRecipientPubkeys({
  currentPubkey,
  mentionPubkeys,
  parentAuthorPubkey,
}: {
  currentPubkey: string;
  mentionPubkeys: readonly string[];
  parentAuthorPubkey: string | null | undefined;
}): string[] {
  const parent = parentAuthorPubkey?.trim().toLowerCase() ?? "";
  const normalized = normalizeMentionPubkeys(
    [...mentionPubkeys],
    currentPubkey,
  );
  const reservesAddressingSlot =
    parent.length > 0 &&
    parent !== currentPubkey.toLowerCase() &&
    !normalized.includes(parent);
  const cap = reservesAddressingSlot ? MENTION_TAG_CAP - 1 : MENTION_TAG_CAP;
  return normalized.length <= cap ? normalized : normalized.slice(0, cap);
}
