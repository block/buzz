import { verifyEvent } from "nostr-tools/pure";

import type { RelayEvent } from "@/shared/api/types";
import type { RelaySubscriptionFilter } from "@/shared/api/relayClientShared";
import { KIND_AGENT_ACTIVITY_FRAME } from "@/shared/constants/kinds";
import { normalizePubkey } from "@/shared/lib/pubkey";

export const AGENT_ACTIVITY_MAX_ITEMS = 32;
export const AGENT_ACTIVITY_MAX_FRAME_BYTES = 4_096;
export const AGENT_ACTIVITY_MAX_DURATION_MS = 7 * 24 * 60 * 60 * 1_000;
export const AGENT_ACTIVITY_MAX_TOKEN_COUNT = 1_000_000_000_000;
export const AGENT_ACTIVITY_FRESHNESS_SECONDS = 300;
export const SHARED_AGENT_ACTIVITY_RETENTION = 200;

const PUBKEY_RE = /^[0-9a-f]{64}$/;
const UUID_RE =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const RFC3339_RE =
  /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{1,9})?(?:Z|[+-]\d{2}:\d{2})$/;
const TERMINAL_STATUSES = new Set(["completed", "failed", "cancelled"]);
const ACTIVITY_CLASSES = new Set(["turn", "tool", "usage"]);
const ACTIVITY_STATUSES = new Set([
  "started",
  "pending",
  "running",
  "completed",
  "failed",
  "cancelled",
]);
const TOOL_KINDS = new Set([
  "read",
  "edit",
  "delete",
  "move",
  "search",
  "execute",
  "think",
  "fetch",
  "switch_mode",
  "other",
]);
const ACTIVITY_KEYS = new Set([
  "activityId",
  "occurredAt",
  "activityClass",
  "status",
  "toolKind",
  "durationMs",
  "usage",
]);
const USAGE_KEYS = new Set([
  "inputTokens",
  "outputTokens",
  "totalTokens",
  "cacheReadTokens",
  "cacheWriteTokens",
]);

export type SharedAgentActivityClass = "turn" | "tool" | "usage";
export type SharedAgentActivityStatus =
  | "started"
  | "pending"
  | "running"
  | "completed"
  | "failed"
  | "cancelled";
export type SharedAgentActivityToolKind =
  | "read"
  | "edit"
  | "delete"
  | "move"
  | "search"
  | "execute"
  | "think"
  | "fetch"
  | "switch_mode"
  | "other";

export type SharedAgentActivityUsage = {
  inputTokens?: number;
  outputTokens?: number;
  totalTokens?: number;
  cacheReadTokens?: number;
  cacheWriteTokens?: number;
};

export type SharedAgentActivity = {
  activityId: string;
  occurredAt: string;
  activityClass: SharedAgentActivityClass;
  status: SharedAgentActivityStatus;
  toolKind?: SharedAgentActivityToolKind;
  durationMs?: number;
  usage?: SharedAgentActivityUsage;
};

export type SharedAgentActivityFrame = {
  version: 1;
  activities: SharedAgentActivity[];
};

export type AgentActivityMode = "owner" | "shared" | "unavailable";

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function hasOnlyKeys(
  value: Record<string, unknown>,
  allowed: ReadonlySet<string>,
) {
  return Object.keys(value).every((key) => allowed.has(key));
}

function isBoundedInteger(value: unknown, max: number): value is number {
  return (
    Number.isSafeInteger(value) &&
    (value as number) >= 0 &&
    (value as number) <= max
  );
}

function isCanonicalUuid(value: unknown): value is string {
  return typeof value === "string" && UUID_RE.test(value);
}

function isRfc3339(value: unknown): value is string {
  return (
    typeof value === "string" &&
    RFC3339_RE.test(value) &&
    Number.isFinite(Date.parse(value))
  );
}

function parseUsage(value: unknown): SharedAgentActivityUsage | null {
  if (!isRecord(value) || !hasOnlyKeys(value, USAGE_KEYS)) return null;
  const entries = Object.entries(value);
  if (entries.length === 0) return null;
  for (const [, count] of entries) {
    if (!isBoundedInteger(count, AGENT_ACTIVITY_MAX_TOKEN_COUNT)) return null;
  }
  return value as SharedAgentActivityUsage;
}

function parseActivity(value: unknown): SharedAgentActivity | null {
  if (!isRecord(value) || !hasOnlyKeys(value, ACTIVITY_KEYS)) return null;
  if (!isCanonicalUuid(value.activityId) || !isRfc3339(value.occurredAt))
    return null;
  if (
    typeof value.activityClass !== "string" ||
    !ACTIVITY_CLASSES.has(value.activityClass)
  )
    return null;
  if (typeof value.status !== "string" || !ACTIVITY_STATUSES.has(value.status))
    return null;

  const hasToolKind = Object.hasOwn(value, "toolKind");
  const hasDuration = Object.hasOwn(value, "durationMs");
  const hasUsage = Object.hasOwn(value, "usage");
  if (
    hasToolKind &&
    (typeof value.toolKind !== "string" || !TOOL_KINDS.has(value.toolKind))
  )
    return null;
  if (
    hasDuration &&
    !isBoundedInteger(value.durationMs, AGENT_ACTIVITY_MAX_DURATION_MS)
  )
    return null;
  if (hasDuration && !TERMINAL_STATUSES.has(value.status)) return null;

  let usage: SharedAgentActivityUsage | undefined;
  if (hasUsage) {
    const parsed = parseUsage(value.usage);
    if (!parsed) return null;
    usage = parsed;
  }

  if (value.activityClass === "turn") {
    if (hasToolKind || hasUsage || value.status === "pending") return null;
  } else if (value.activityClass === "tool") {
    if (!hasToolKind || hasUsage || value.status === "started") return null;
  } else {
    if (value.status !== "completed" || hasToolKind || hasDuration || !usage)
      return null;
  }

  return {
    activityId: value.activityId,
    occurredAt: value.occurredAt,
    activityClass: value.activityClass as SharedAgentActivityClass,
    status: value.status as SharedAgentActivityStatus,
    ...(hasToolKind
      ? { toolKind: value.toolKind as SharedAgentActivityToolKind }
      : {}),
    ...(hasDuration ? { durationMs: value.durationMs as number } : {}),
    ...(usage ? { usage } : {}),
  };
}

function parseFrame(content: string): SharedAgentActivityFrame | null {
  if (
    new TextEncoder().encode(content).byteLength >
    AGENT_ACTIVITY_MAX_FRAME_BYTES
  )
    return null;
  let value: unknown;
  try {
    value = JSON.parse(content);
  } catch {
    return null;
  }
  if (
    !isRecord(value) ||
    !hasOnlyKeys(value, new Set(["version", "activities"]))
  )
    return null;
  if (value.version !== 1 || !Array.isArray(value.activities)) return null;
  if (
    value.activities.length < 1 ||
    value.activities.length > AGENT_ACTIVITY_MAX_ITEMS
  )
    return null;
  const activities: SharedAgentActivity[] = [];
  for (const item of value.activities) {
    const parsed = parseActivity(item);
    if (!parsed) return null;
    activities.push(parsed);
  }
  return { version: 1, activities };
}

export function buildAgentActivitySummaryFilter(
  agentPubkey: string,
  channelId: string,
): RelaySubscriptionFilter {
  return {
    kinds: [KIND_AGENT_ACTIVITY_FRAME],
    authors: [normalizePubkey(agentPubkey)],
    "#h": [channelId.toLowerCase()],
    limit: 0,
  };
}

export function parseAgentActivityEvent(
  event: RelayEvent,
  input: {
    expectedAgentPubkey: string;
    expectedChannelId: string;
    nowSeconds?: number;
  },
): SharedAgentActivityFrame | null {
  const expectedAgent = normalizePubkey(input.expectedAgentPubkey);
  const expectedChannel = input.expectedChannelId.toLowerCase();
  const nowSeconds = input.nowSeconds ?? Math.floor(Date.now() / 1_000);

  if (!PUBKEY_RE.test(expectedAgent) || !UUID_RE.test(expectedChannel))
    return null;
  if (
    event.kind !== KIND_AGENT_ACTIVITY_FRAME ||
    event.pubkey !== expectedAgent ||
    !Number.isSafeInteger(event.created_at) ||
    Math.abs(event.created_at - nowSeconds) >
      AGENT_ACTIVITY_FRESHNESS_SECONDS ||
    !Array.isArray(event.tags) ||
    event.tags.length !== 2
  )
    return null;

  const hTags = event.tags.filter(
    (tag) => Array.isArray(tag) && tag[0] === "h",
  );
  const agentTags = event.tags.filter(
    (tag) => Array.isArray(tag) && tag[0] === "agent",
  );
  if (
    hTags.length !== 1 ||
    agentTags.length !== 1 ||
    hTags[0].length !== 2 ||
    agentTags[0].length !== 2 ||
    hTags[0][1] !== expectedChannel ||
    agentTags[0][1] !== expectedAgent ||
    agentTags[0][1] !== event.pubkey
  )
    return null;

  const frame = parseFrame(event.content);
  if (!frame) return null;
  try {
    // nostr-tools memoizes verification on the object. Verify a fresh canonical
    // envelope so an object mutated after an earlier successful check cannot
    // inherit that cached result.
    const canonicalEvent = {
      id: event.id,
      pubkey: event.pubkey,
      created_at: event.created_at,
      kind: event.kind,
      tags: event.tags.map((tag) => [...tag]),
      content: event.content,
      sig: event.sig,
    };
    return verifyEvent(canonicalEvent) ? frame : null;
  } catch {
    return null;
  }
}

export function mergeSharedAgentActivities(
  current: readonly SharedAgentActivity[],
  incoming: readonly SharedAgentActivity[],
  maxItems = SHARED_AGENT_ACTIVITY_RETENTION,
): SharedAgentActivity[] {
  const byId = new Map(current.map((item) => [item.activityId, item]));
  for (const item of incoming) byId.set(item.activityId, item);
  return [...byId.values()]
    .sort((left, right) => {
      const time = Date.parse(left.occurredAt) - Date.parse(right.occurredAt);
      return time || left.activityId.localeCompare(right.activityId);
    })
    .slice(-Math.max(0, maxItems));
}

export function resolveAgentActivityMode(input: {
  agentOwnerPubkey: string | null | undefined;
  currentPubkey: string | null | undefined;
  channel: Pick<
    import("@/shared/api/types").Channel,
    "id" | "channelType" | "isMember"
  > | null;
}): AgentActivityMode {
  if (
    input.agentOwnerPubkey &&
    input.currentPubkey &&
    normalizePubkey(input.agentOwnerPubkey) ===
      normalizePubkey(input.currentPubkey)
  )
    return "owner";
  if (
    input.channel?.isMember &&
    (input.channel.channelType === "stream" ||
      input.channel.channelType === "forum")
  )
    return "shared";
  return "unavailable";
}

export function describeSharedAgentActivity(activity: SharedAgentActivity): {
  label: string;
  detail: string;
} {
  const status =
    activity.status === "pending"
      ? "Pending"
      : activity.status === "started" || activity.status === "running"
        ? "In progress"
        : activity.status === "completed"
          ? "Completed"
          : activity.status === "failed"
            ? "Failed"
            : "Cancelled";

  if (activity.activityClass === "usage") {
    const total = activity.usage?.totalTokens;
    return {
      label: "Usage updated",
      detail: total === undefined ? status : `${total.toLocaleString()} tokens`,
    };
  }
  if (activity.activityClass === "turn") {
    const labels: Record<SharedAgentActivityStatus, string> = {
      started: "Working",
      pending: "Working",
      running: "Working",
      completed: "Turn completed",
      failed: "Turn failed",
      cancelled: "Turn cancelled",
    };
    return { label: labels[activity.status], detail: status };
  }
  const labels: Record<SharedAgentActivityToolKind, string> = {
    read: "Reading",
    edit: "Editing",
    delete: "Deleting",
    move: "Moving",
    search: "Searching",
    execute: "Running a tool",
    think: "Working",
    fetch: "Fetching",
    switch_mode: "Switching mode",
    other: "Running a tool",
  };
  return { label: labels[activity.toolKind ?? "other"], detail: status };
}
