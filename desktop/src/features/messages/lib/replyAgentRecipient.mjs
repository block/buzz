const normalizePubkey = (pubkey) => (pubkey ?? "").trim().toLowerCase();

/**
 * The agent a reply answers, when the parent message was written by one.
 *
 * Agents subscribe to the events that tag them, so a reply without a `p` tag
 * for the agent never reaches it. Replying to an agent's message is addressed
 * to that agent, so the reply tags it as if it were mentioned. An agent's
 * message carries its NIP-OA owner attestation (`auth` tag); people's messages
 * do not, so replies to people stay untagged.
 *
 * Returns null when the parent is not cached, is not agent-authored, or was
 * written by the sender.
 */
export function replyAgentRecipient(
  parentEventId,
  messageCaches,
  senderPubkey,
) {
  for (const messages of messageCaches) {
    const parent = messages.find((event) => event.id === parentEventId);
    if (!parent) continue;
    const author = normalizePubkey(parent.pubkey);
    const isAgent = (parent.tags ?? []).some((tag) => tag[0] === "auth");
    return isAgent && author && author !== normalizePubkey(senderPubkey)
      ? author
      : null;
  }
  return null;
}
