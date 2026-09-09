import type { PulseConversation } from "./unifiedFeed";

type PriorityKind = "agent" | "dm" | "mention" | "thread";
export type BriefingKind = PriorityKind | `recent:${string}`;
export type BriefingGroup = {
  kind: BriefingKind;
  label: string;
  ids: Set<string>;
};
export type ReadSignals = {
  getChannelReadAt: (id: string) => number | null;
  getThreadReadAt: (id: string, channelId?: string | null) => number | null;
  getMessageReadAt: (id: string) => number | null;
  isFollowingThread: (id: string) => boolean;
  isThreadMuted: (id: string) => boolean;
};

// Only explicit first-person requests qualify. Quoted text and code are not
// evidence of the posting agent's state; this is a message signal, not liveness.
function asksForInput(body: string) {
  const prose = body
    .replace(/```[\s\S]*?```/g, "")
    .split("\n")
    .filter((line) => !line.trim().startsWith(">"))
    .join("\n");
  return /(?:^|[.!?]\s+|\n)\s*(?:i(?:'m| am) blocked (?:on|by|until)|blocked (?:on|by|until)|i need your (?:approval|input|help|decision|permission)|(?:i(?:'m| am) )?waiting for your (?:approval|input|decision|permission))\b/i.test(
    prose,
  );
}

/** Summarize recent, personally relevant signals from the loaded feed. */
export function buildPulseBriefing(
  conversations: PulseConversation[],
  currentPubkey: string | undefined,
  reads: ReadSignals,
  now: number,
): BriefingGroup[] {
  const me = currentPubkey?.toLowerCase();
  if (!me) return [];
  const buckets: Record<PriorityKind, PulseConversation[]> = {
    agent: [],
    dm: [],
    mention: [],
    thread: [],
  };
  for (const item of conversations) {
    if (reads.isThreadMuted(item.rootId)) continue;
    const recent = item.messages.filter(
      (m) =>
        !m.pending && m.createdAt >= now - 48 * 3600 && m.createdAt <= now + 60,
    );
    const incoming = recent.filter(
      (m) => m.pubkey && m.pubkey.toLowerCase() !== me,
    );
    const unread = incoming.filter((m) => {
      const frontier = Math.max(
        reads.getChannelReadAt(item.channel?.id ?? "") ?? 0,
        reads.getThreadReadAt(item.rootId, item.channel?.id) ?? 0,
        reads.getMessageReadAt(m.id) ?? 0,
      );
      return m.createdAt > frontier;
    });
    const isDm = item.channel?.channelType === "dm";
    if (isDm && unread.length) buckets.dm.push(item);
    const mentionsMe = (tags?: string[][]) =>
      tags?.some((tag) => tag[0] === "p" && tag[1]?.toLowerCase() === me);
    if (!isDm && unread.some((m) => mentionsMe(m.tags)))
      buckets.mention.push(item);
    if (!isDm && unread.length && reads.isFollowingThread(item.rootId))
      buckets.thread.push(item);
    const latest = recent.at(-1);
    if (
      latest?.isAgent &&
      latest.pubkey?.toLowerCase() !== me &&
      (isDm ||
        latest.ownerPubkey?.toLowerCase() === me ||
        mentionsMe(latest.tags) ||
        item.messages.some((m) => m.pubkey?.toLowerCase() === me)) &&
      asksForInput(latest.body)
    )
      buckets.agent.push(item);
  }
  const dmCount = new Set(buckets.dm.map((item) => item.channel?.id)).size;
  const agents = new Map(
    buckets.agent.map((item) => {
      const message = item.messages.filter((m) => !m.pending).at(-1);
      return [message?.pubkey?.toLowerCase(), message?.author];
    }),
  );
  const labels: Record<PriorityKind, string> = {
    agent:
      agents.size === 1
        ? `${[...agents.values()][0]} may need your input`
        : `${agents.size} agents may need your input`,
    dm: `${dmCount} DM conversation${dmCount === 1 ? " has" : "s have"} unread messages`,
    mention: `you’re mentioned in ${buckets.mention.length} unread thread${buckets.mention.length === 1 ? "" : "s"}`,
    thread: `${buckets.thread.length} followed thread${buckets.thread.length === 1 ? " has" : "s have"} updates`,
  };
  return (["agent", "dm", "mention", "thread"] as const)
    .filter((kind) => buckets[kind].length > 0)
    .map((kind) => ({
      kind,
      label: labels[kind],
      ids: new Set(buckets[kind].map((item) => item.id)),
    }));
}

/** Higher-consequence conversations precede routine activity. */
export function briefingPriority(id: string, groups: BriefingGroup[]) {
  const weights: Record<string, number> = {
    agent: 4,
    dm: 3,
    mention: 2,
    thread: 1,
  };
  return Math.max(
    0,
    ...groups
      .filter((group) => group.ids.has(id))
      .map((group) => weights[group.kind] ?? 0),
  );
}
