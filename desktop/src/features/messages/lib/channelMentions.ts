import type { PresenceLookup } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { hasMention } from "./hasMention";

export const CHANNEL_MENTION_RECIPIENT_MARKER_PREFIX = "buzz:audience:";
export const CHANNEL_MENTION_REFERENCE_TAG = "buzz-audience-ref";
export const MAX_CHANNEL_MENTION_RECIPIENTS = 4_000;

export type ChannelMentionMode = "everyone" | "here";

export const CHANNEL_MENTION_SUGGESTIONS: ReadonlyArray<{
  annotation: string;
  displayName: ChannelMentionMode;
}> = [
  {
    displayName: "everyone",
    annotation: "Notify everyone in this channel",
  },
  {
    displayName: "here",
    annotation: "Notify everyone online in this channel",
  },
];

export function channelMentionModes(text: string): ChannelMentionMode[] {
  return CHANNEL_MENTION_SUGGESTIONS.flatMap(({ displayName }) =>
    hasMention(text, displayName) ? [displayName] : [],
  );
}

export function getChannelMentionRecipient(tag: readonly string[]) {
  if (
    tag.length !== 4 ||
    tag[0] !== "p" ||
    !tag[1] ||
    tag[2] !== "" ||
    (tag[3] !== "buzz:audience:everyone" && tag[3] !== "buzz:audience:here")
  ) {
    return null;
  }

  return normalizePubkey(tag[1]);
}

export function isChannelMentionRecipientTag(tag: readonly string[]) {
  return getChannelMentionRecipient(tag) !== null;
}

export function isChannelWideMentionEvent(tags: readonly string[][]) {
  return tags.some(isChannelMentionRecipientTag);
}

export function getChannelMentionAudienceLimitError(recipientCount: number) {
  return recipientCount > MAX_CHANNEL_MENTION_RECIPIENTS
    ? `Channel-wide mentions support up to ${MAX_CHANNEL_MENTION_RECIPIENTS.toLocaleString()} recipients.`
    : null;
}

export function hasChannelMentionForPubkey(
  tags: readonly string[][],
  pubkey: string,
) {
  const normalized = normalizePubkey(pubkey);
  return (
    normalized.length > 0 &&
    tags.some((tag) => getChannelMentionRecipient(tag) === normalized)
  );
}

export function buildChannelMentionTags(input: {
  memberPubkeys: Iterable<string>;
  originalText?: string;
  presence?: PresenceLookup;
  selfPubkey: string;
  text: string;
}): string[][] {
  const modes = channelMentionModes(input.text);
  if (modes.length === 0) return [];

  const originalModes = new Set(channelMentionModes(input.originalText ?? ""));
  const referenceTags = modes.map((mode) => [
    CHANNEL_MENTION_REFERENCE_TAG,
    mode,
  ]);
  const memberPubkeys = [
    ...new Set([...input.memberPubkeys].map(normalizePubkey)),
  ]
    .filter(Boolean)
    .filter((pubkey) => pubkey !== normalizePubkey(input.selfPubkey));

  // @everyone is the superset. If both reserved mentions appear, one exact
  // recipient snapshot is enough and avoids duplicate notification rows.
  const notifyModes = modes.includes("everyone")
    ? ["everyone" as const]
    : modes;
  const recipientTags = notifyModes.flatMap((mode) => {
    if (originalModes.has(mode)) return [];
    const recipients =
      mode === "everyone"
        ? memberPubkeys
        : memberPubkeys.filter(
            (pubkey) => input.presence?.[pubkey] === "online",
          );
    return recipients.map((pubkey) => [
      "p",
      pubkey,
      "",
      `${CHANNEL_MENTION_RECIPIENT_MARKER_PREFIX}${mode}`,
    ]);
  });

  return [...referenceTags, ...recipientTags];
}
