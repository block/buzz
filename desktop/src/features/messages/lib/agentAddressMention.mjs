import { normalizePubkey } from "../../../shared/lib/pubkey.ts";

export const AGENT_ADDRESS_MENTION_MARKER = "agent-address";

/**
 * Persist the subset of delivered mentions that came from the composer's
 * address tray. The ordinary `p` tag remains the notification mechanism;
 * this annotated reference is display metadata for reconstructing the tray
 * state when the message is rendered later.
 */
export function buildAgentAddressMentionTags(
  addressedPubkeys,
  deliveredPubkeys,
) {
  const delivered = new Set([...deliveredPubkeys].map(normalizePubkey));
  return [...new Set([...addressedPubkeys].map(normalizePubkey))]
    .filter((pubkey) => pubkey && delivered.has(pubkey))
    .map((pubkey) => ["mention", pubkey, AGENT_ADDRESS_MENTION_MARKER]);
}

/** Return the ordered, deduplicated agent-address recipients on an event. */
export function getAgentAddressMentionPubkeys(tags) {
  return [
    ...new Set(
      (tags ?? [])
        .filter(
          (tag) =>
            tag[0] === "mention" &&
            tag[2] === AGENT_ADDRESS_MENTION_MARKER &&
            Boolean(tag[1]),
        )
        .map((tag) => normalizePubkey(tag[1])),
    ),
  ];
}

export function isAgentAddressMentionTag(tag) {
  return tag[0] === "mention" && tag[2] === AGENT_ADDRESS_MENTION_MARKER;
}
