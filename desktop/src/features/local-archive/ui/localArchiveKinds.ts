import {
  CHANNEL_AUX_EVENT_KINDS,
  CHANNEL_MESSAGE_EVENT_KINDS,
  KIND_AGENT_OBSERVER_FRAME,
  KIND_HUDDLE_ENDED,
  KIND_HUDDLE_PARTICIPANT_JOINED,
  KIND_HUDDLE_PARTICIPANT_LEFT,
  KIND_HUDDLE_STARTED,
  KIND_STREAM_MESSAGE_DIFF,
  KIND_SYSTEM_MESSAGE,
} from "@/shared/constants/kinds";

// ── Kind groups (derived from shared constants — never raw literals) ──────────

export type KindGroup = {
  /** Group label shown as a section header. */
  label: string;
  /** Individual kinds in this group, each with a human-readable name. */
  items: ReadonlyArray<{ kind: number; label: string }>;
};

/**
 * Ordered list of kind groups presented in the Step 2 checklist.
 * Every item derives its kind value from a named constant in kinds.ts.
 */
export const KIND_GROUPS: ReadonlyArray<KindGroup> = [
  {
    label: "Messages & posts",
    items: [
      ...CHANNEL_MESSAGE_EVENT_KINDS.map((k) => ({
        kind: k,
        label: kindLabel(k),
      })),
      { kind: KIND_STREAM_MESSAGE_DIFF, label: "Message diffs (kind 40008)" },
    ],
  },
  {
    label: "Reactions, edits & deletions",
    items: CHANNEL_AUX_EVENT_KINDS.map((k) => ({
      kind: k,
      label: kindLabel(k),
    })),
  },
  {
    label: "Huddle events",
    items: [
      { kind: KIND_HUDDLE_STARTED, label: "Huddle started" },
      { kind: KIND_HUDDLE_PARTICIPANT_JOINED, label: "Participant joined" },
      { kind: KIND_HUDDLE_PARTICIPANT_LEFT, label: "Participant left" },
      { kind: KIND_HUDDLE_ENDED, label: "Huddle ended" },
    ],
  },
  {
    label: "System messages",
    items: [
      { kind: KIND_SYSTEM_MESSAGE, label: "System messages (kind 40099)" },
    ],
  },
] as const;

/** Human-readable label for a known kind number. */
function kindLabel(kind: number): string {
  switch (kind) {
    case 5:
      return "Event deletions (kind 5)";
    case 7:
      return "Reactions (kind 7)";
    case 9:
      return "Stream messages (kind 9)";
    case 9005:
      return "Buzz-native deletions (kind 9005)";
    case 40002:
      return "Stream messages v2 (kind 40002)";
    case 40003:
      return "Message edits (kind 40003)";
    case 45001:
      return "Forum posts (kind 45001)";
    case 45003:
      return "Forum comments (kind 45003)";
    default:
      return `Kind ${kind}`;
  }
}

/** All kinds covered by KIND_GROUPS, as a flat sorted set for dedup checks. */
const GROUPED_KINDS: ReadonlySet<number> = new Set(
  KIND_GROUPS.flatMap((g) => g.items.map((i) => i.kind)),
);

// ── Custom-kinds parser ───────────────────────────────────────────────────────

export type ParsedCustomKinds = {
  /** Valid non-negative integer kind numbers not already in KIND_GROUPS. */
  valid: number[];
  /**
   * Tokens that failed validation (not a non-negative integer, or duplicate
   * of a grouped kind), returned so the UI can show inline feedback.
   */
  invalid: string[];
};

/**
 * Parse a free-text custom kinds input.
 *
 * Rules:
 * - Split on whitespace and/or commas.
 * - Accept non-negative integers in the valid NIP-01 kind range 0..=65535 only
 *   (no floats, no negatives, no hex, no values > 65535).
 * - Reject tokens that duplicate a kind already present in KIND_GROUPS.
 * - Deduplicate valid tokens (keep first occurrence).
 * - Return both valid numbers and invalid tokens for inline feedback.
 */
export function parseCustomKinds(raw: string): ParsedCustomKinds {
  const tokens = raw.split(/[\s,]+/).filter((t) => t.length > 0);
  const valid: number[] = [];
  const invalid: string[] = [];
  const seen = new Set<number>();

  for (const token of tokens) {
    // Must be a non-negative integer with no leading sign, decimal point, etc.
    if (!/^\d+$/.test(token)) {
      invalid.push(token);
      continue;
    }
    const n = parseInt(token, 10);
    if (!Number.isFinite(n) || n < 0 || n > 65535) {
      invalid.push(token);
      continue;
    }
    if (GROUPED_KINDS.has(n)) {
      // Already available in the checklist — silently ignore (not an error).
      continue;
    }
    if (seen.has(n)) {
      continue; // deduplicate
    }
    seen.add(n);
    valid.push(n);
  }

  return { valid, invalid };
}

// ── Final kinds computation ───────────────────────────────────────────────────

/**
 * Compute the final sorted, deduped kinds array to pass to
 * `createSaveSubscription` for a channel_h subscription.
 *
 * @param checkedKinds - Set of kind numbers checked via the group checklist.
 * @param customKinds  - Valid custom kind numbers from `parseCustomKinds`.
 */
export function buildFinalKinds(
  checkedKinds: ReadonlySet<number>,
  customKinds: ReadonlyArray<number>,
): number[] {
  const merged = new Set<number>([...checkedKinds, ...customKinds]);
  return [...merged].sort((a, b) => a - b);
}

// ── Group-toggle helpers ──────────────────────────────────────────────────────

/** Returns whether all kinds in a group are in `checkedKinds`. */
export function isGroupFullyChecked(
  group: KindGroup,
  checkedKinds: ReadonlySet<number>,
): boolean {
  return group.items.every((i) => checkedKinds.has(i.kind));
}

/** Returns whether some (but not all) kinds in a group are in `checkedKinds`. */
export function isGroupIndeterminate(
  group: KindGroup,
  checkedKinds: ReadonlySet<number>,
): boolean {
  const some = group.items.some((i) => checkedKinds.has(i.kind));
  const all = group.items.every((i) => checkedKinds.has(i.kind));
  return some && !all;
}

/**
 * Toggle all kinds in a group: if the group is fully checked, uncheck all;
 * otherwise check all. Returns the updated set (new object).
 */
export function toggleGroup(
  group: KindGroup,
  checkedKinds: ReadonlySet<number>,
): Set<number> {
  const next = new Set(checkedKinds);
  if (isGroupFullyChecked(group, checkedKinds)) {
    for (const item of group.items) {
      next.delete(item.kind);
    }
  } else {
    for (const item of group.items) {
      next.add(item.kind);
    }
  }
  return next;
}

/** Toggle a single kind in `checkedKinds`. Returns the updated set. */
export function toggleKind(
  kind: number,
  checkedKinds: ReadonlySet<number>,
): Set<number> {
  const next = new Set(checkedKinds);
  if (next.has(kind)) {
    next.delete(kind);
  } else {
    next.add(kind);
  }
  return next;
}

// ── Subscription request builder ──────────────────────────────────────────────

export type SubscriptionRequest = {
  scopeType: "channel_h" | "owner_p";
  scopeValue: string;
  kinds: number[];
};

/**
 * Build the final subscription request to pass to `createSaveSubscription`.
 *
 * - For `owner_p`: kinds is always `[KIND_AGENT_OBSERVER_FRAME]`, scopeValue is
 *   the identity pubkey.
 * - For `channel_h`: kinds is the sorted deduped union of checkedKinds and
 *   customKinds; scopeValue is the channel id.
 *
 * Returns `null` when the request would be invalid (empty pubkey for owner_p,
 * empty channelId or zero kinds for channel_h).
 */
export function buildSubscriptionRequest(
  source: "channel_h" | "owner_p",
  scopeValue: string,
  checkedKinds: ReadonlySet<number>,
  customKinds: ReadonlyArray<number>,
): SubscriptionRequest | null {
  if (!scopeValue) return null;
  if (source === "owner_p") {
    return {
      scopeType: "owner_p",
      scopeValue,
      kinds: [KIND_AGENT_OBSERVER_FRAME],
    };
  }
  const kinds = buildFinalKinds(checkedKinds, customKinds);
  if (kinds.length === 0) return null;
  return { scopeType: "channel_h", scopeValue, kinds };
}
