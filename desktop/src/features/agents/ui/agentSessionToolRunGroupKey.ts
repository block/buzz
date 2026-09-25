import type {
  TranscriptDisplayBlock,
  TranscriptToolRunSummary,
} from "./agentSessionTranscriptGrouping";

/**
 * Candidate identities for a tool-run group — one per leaf, in arrival order.
 *
 * A summary's own `id` encodes how the group was batched at the moment it was
 * built: a same-kind run is `summary:read_file:<first>`, and the moment one
 * differing tool call lands beside it the same work is rebuilt as
 * `summary:mixed:<first>`. Keying reader state on that id would silently
 * discard a deliberate choice to open a group every time the agent emitted one
 * more call.
 *
 * Leaf item ids are the stable part: they derive from the channel and the ACP
 * tool-call id, so they survive re-derivation over merged live/archive windows.
 * `turnId` scopes each candidate so two turns can never collide, and is read
 * from the leaf rather than the summary because only leaves carry turn
 * identity.
 *
 * Every leaf is returned rather than just the first because leaf membership is
 * not monotonic: a call that fails is ejected from the group (see
 * `isGroupingEligible`), and when the ejected call was the FIRST leaf the group
 * needs a way to recognise itself under a new leading leaf. Resolving that is
 * the job of `resolveToolRunGroupKey`, which remembers which candidates already
 * belong to a live identity.
 */
export function getToolRunGroupMemberKeys(
  summary: Pick<TranscriptToolRunSummary, "id" | "items">,
): string[] {
  if (summary.items.length === 0) {
    // A group with no leaves cannot be identified by content; fall back to the
    // batch-derived id rather than colliding every empty group onto one key.
    return [`group:empty:${summary.id}`];
  }
  return summary.items.map(
    (item) => `group:${item.turnId ?? "no-turn"}:${item.id}`,
  );
}

/**
 * The group's anchor candidate: the identity a group takes when nothing is
 * remembered about it yet.
 *
 * Callers that need reader state to survive re-grouping must go through
 * {@link resolveToolRunGroupIdentities} instead — this value alone churns when
 * the leading leaf leaves the group.
 */
export function getToolRunGroupKey(
  summary: Pick<TranscriptToolRunSummary, "id" | "items">,
): string {
  return getToolRunGroupMemberKeys(summary)[0];
}

/**
 * Remembered mapping from a leaf candidate key to the durable identity of the
 * group that leaf belonged to.
 */
export type ToolRunGroupIdentityTable = ReadonlyMap<string, string>;

export type ToolRunGroupIdentityResolution = {
  /** Durable identity per input group, in input order. */
  keys: string[];
  /**
   * Table to remember for the next pass. Contains only leaves present in this
   * pass: continuity is carried by the leaves that stayed, so remembering
   * departed ones would grow without bound for no benefit.
   */
  table: Map<string, string>;
};

/**
 * Assign a durable identity to every group in one grouping pass.
 *
 * Grouping is re-derived from scratch on every transcript pass, so a group has
 * no identity of its own — only its leaves do. This resolves the two together:
 * a group keeps the identity any of its current leaves already carried, and
 * only invents a new one when none of them is recognised. That is what makes a
 * reader's collapse survive the leading tool call being ejected after it fails
 * (`isGroupingEligible` drops failures, so the next leaf becomes first and the
 * anchor candidate changes).
 *
 * Two rules keep it honest:
 *  - An identity is claimed by at most one group per pass. When a run splits in
 *    the middle — a failure between two eligible stretches — the part holding
 *    the earlier leaves keeps the identity and the remainder starts fresh,
 *    rather than both halves sharing (and fighting over) one reader state.
 *  - Groups are visited in transcript order purely to break that claim tie
 *    deterministically. Position never becomes part of an identity: every
 *    identity is a content-derived leaf key, so archive pages prepending older
 *    turns cannot renumber anything.
 *
 * Pure and idempotent: re-running with the returned table yields the same keys,
 * so a repeated render can never churn identities.
 */
export function resolveToolRunGroupIdentities(
  groups: readonly (readonly string[])[],
  previous: ToolRunGroupIdentityTable,
): ToolRunGroupIdentityResolution {
  const table = new Map<string, string>();
  const claimed = new Set<string>();
  const keys: string[] = [];

  for (const memberKeys of groups) {
    let resolved: string | undefined;
    for (const memberKey of memberKeys) {
      const remembered = previous.get(memberKey);
      if (remembered !== undefined && !claimed.has(remembered)) {
        resolved = remembered;
        break;
      }
    }
    // Nothing recognised (or every match is already spoken for): anchor on this
    // group's own first leaf. A group with no leaves still has its batch id as
    // a candidate, so `memberKeys` is never empty.
    if (resolved === undefined) {
      resolved = memberKeys[0];
    }
    // A fresh anchor can itself collide with an identity another group in this
    // pass already claimed (two groups whose leaves overlap cannot both be it).
    // Fall through to the next unclaimed candidate so distinct groups never
    // share one reader state.
    if (claimed.has(resolved)) {
      resolved =
        memberKeys.find((memberKey) => !claimed.has(memberKey)) ?? resolved;
    }
    claimed.add(resolved);
    keys.push(resolved);
    for (const memberKey of memberKeys) {
      table.set(memberKey, resolved);
    }
  }

  return { keys, table };
}

/**
 * Every group a transcript will render, in reading order, as
 * `[summary id, member candidate keys]`.
 *
 * Nested same-kind summaries are included because a mixed burst renders them as
 * their own group cards, and each therefore owns reader state of its own.
 * Summary ids are unique within a transcript (each embeds the id of its own
 * first leaf), so they are safe as the lookup handle for the resolved identity.
 */
function collectToolRunGroupMembers(
  blocks: readonly TranscriptDisplayBlock[],
): Array<[string, string[]]> {
  const collected: Array<[string, string[]]> = [];

  const walk = (summary: TranscriptToolRunSummary): void => {
    collected.push([summary.id, getToolRunGroupMemberKeys(summary)]);
    for (const child of summary.segments ?? []) {
      if (child.kind === "summary") walk(child.summary);
    }
  };

  for (const block of blocks) {
    if (block.kind !== "turn") continue;
    for (const segment of block.segments) {
      if (segment.kind === "summary") walk(segment.summary);
    }
  }

  return collected;
}

export type TranscriptToolRunGroupKeyResolution = {
  /** Durable identity for each rendered group, by summary id. */
  keysBySummaryId: Map<string, string>;
  /** Table to remember for the next pass. */
  table: Map<string, string>;
};

/**
 * Scope reader state to one rendered transcript.
 *
 * Tool-call ids are agent-supplied and can repeat across agents, while a channel
 * can render the full viewer and compact preview together. A length-prefixed
 * scope keeps those surfaces independent without making position part of the
 * group's identity.
 */
export function scopeToolRunGroupKey(scope: string, groupKey: string): string {
  return `transcript:${scope.length}:${scope}:${groupKey}`;
}

/**
 * Resolve durable identities for a whole transcript pass.
 *
 * One pass over all groups (rather than per-card resolution) is what lets a
 * group recognise leaves that another group is also claiming: identity is
 * allocated once, in order, with no group able to take an identity a sibling
 * already holds.
 */
export function resolveTranscriptToolRunGroupKeys(
  blocks: readonly TranscriptDisplayBlock[],
  previous: ToolRunGroupIdentityTable,
): TranscriptToolRunGroupKeyResolution {
  const members = collectToolRunGroupMembers(blocks);
  const { keys, table } = resolveToolRunGroupIdentities(
    members.map(([, memberKeys]) => memberKeys),
    previous,
  );

  const keysBySummaryId = new Map<string, string>();
  members.forEach(([summaryId], index) => {
    keysBySummaryId.set(summaryId, keys[index]);
  });

  return { keysBySummaryId, table };
}
