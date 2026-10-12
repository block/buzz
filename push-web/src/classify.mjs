// Decide who gets a push for a relay event and what it says. Payloads carry NO message content:
// only sender name, channel name and a category line, plus a routing target for notificationclick.
export const MENTION_KINDS = [9, 40002, 1, 45001, 45003, 1618, 1619, 1621, 1630, 1631, 1632, 1633];
export const APPROVAL_KINDS = [46010, 46011, 46012];
export const DM_MESSAGE_KINDS = [9, 40002];

const tagVal = (ev, k) => ev.tags.find((t) => t[0] === k)?.[1];

/** @returns {{ category: "dm"|"mention"|"thread_reply"|"needs_action", recipient: string }[]} */
export function recipientsFor(ev, { registered, dmChannelMembers, rootAuthor = null }) {
  const out = [];
  const seen = new Set();
  const add = (recipient, category) => {
    if (!registered.has(recipient) || recipient === ev.pubkey || seen.has(recipient)) return;
    seen.add(recipient);
    out.push({ recipient, category });
  };
  const h = tagVal(ev, "h");
  const pTagged = ev.tags.filter((t) => t[0] === "p" && typeof t[1] === "string").map((t) => t[1]);
  if (APPROVAL_KINDS.includes(ev.kind)) {
    for (const p of pTagged) add(p, "needs_action");
    return out;
  }
  if (h && dmChannelMembers.has(h) && DM_MESSAGE_KINDS.includes(ev.kind)) {
    for (const m of dmChannelMembers.get(h)) add(m, "dm");
    return out;
  }
  // Mentions own any reply that @-tags the user (same rule as desktop, so nothing notifies twice).
  if (MENTION_KINDS.includes(ev.kind)) for (const p of pTagged) add(p, "mention");
  if (rootAuthor && DM_MESSAGE_KINDS.includes(ev.kind)) add(rootAuthor, "thread_reply");
  return out;
}

/** Thread root id for a reply (NIP-10 marked root, else first e tag), or null for top-level messages. */
export function threadRootOf(ev) {
  return ev.tags.find((t) => t[0] === "e" && t[3] === "root")?.[1] ?? ev.tags.find((t) => t[0] === "e")?.[1] ?? null;
}

export function buildPayload(ev, category, { senderName, channelName }) {
  const who = senderName || "Someone";
  const where = channelName ? `#${channelName}` : "";
  const body = {
    dm: "New message",
    mention: where ? `Mentioned you in ${where}` : "Mentioned you",
    thread_reply: where ? `Replied in a thread in ${where}` : "Replied in a thread",
    needs_action: "Needs your approval",
  }[category];
  const root = threadRootOf(ev);
  return {
    title: category === "dm" ? `New message from ${who}` : who,
    body,
    tag: `buzz-${ev.id}`,
    target: {
      channelId: tagVal(ev, "h") ?? null,
      channelName: channelName || null,
      eventId: ev.id,
      kind: ev.kind,
      pubkey: ev.pubkey,
      createdAt: ev.created_at,
      threadRootId: root,
    },
  };
}
