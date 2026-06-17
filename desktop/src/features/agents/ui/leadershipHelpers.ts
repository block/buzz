import type { ObserverEvent } from "./agentSessionTypes";

/** Per-window-instance leadership state derived from `leadership_status` frames. */
export type InstanceLeadership = {
  instanceId: string;
  isLeader: boolean;
  lastSeen: number; // epoch ms — Date.parse(event.timestamp)
};

export const LEADERSHIP_EVENT_KIND = "leadership_status";

/**
 * An instance is stale once it has missed 3 consecutive 5s emit ticks. A
 * surviving instance re-emits within 5s, so 15s tolerates a single dropped
 * relay frame without the badge flickering.
 */
export const LEADERSHIP_STALE_MS = 15_000;

/**
 * Narrows the untrusted `unknown` payload of a `leadership_status` frame.
 * Harness emits arbitrary JSON (`observer.rs`), so the contents are validated
 * here at the boundary; malformed frames are dropped rather than producing
 * `undefined`/`NaN` entries.
 */
export function parseLeadershipPayload(
  payload: unknown,
): { instanceId: string; isLeader: boolean } | null {
  if (typeof payload !== "object" || payload === null) {
    return null;
  }
  const record = payload as Record<string, unknown>;
  if (
    typeof record.instanceId !== "string" ||
    typeof record.isLeader !== "boolean"
  ) {
    return null;
  }
  return { instanceId: record.instanceId, isLeader: record.isLeader };
}

/**
 * Reduces an agent's observer events to the latest `leadership_status` frame
 * per `instanceId`. `events` must be sorted ascending (the store keeps
 * `eventsByAgent` sorted by `compareObserverEvents`), so a simple
 * last-write-wins walk in iteration order is correct — no comparator needed.
 *
 * Instances whose latest frame fell out of the trimmed event window are
 * naturally absent, so this also prunes zombie instanceIds. Frames that fail
 * the payload guard or carry an unparseable timestamp are dropped.
 */
export function buildLeadership(
  events: readonly ObserverEvent[],
): InstanceLeadership[] {
  const latestByInstance = new Map<string, InstanceLeadership>();
  for (const event of events) {
    if (event.kind !== LEADERSHIP_EVENT_KIND) {
      continue;
    }
    const parsed = parseLeadershipPayload(event.payload);
    if (!parsed) {
      continue;
    }
    const lastSeen = Date.parse(event.timestamp);
    if (Number.isNaN(lastSeen)) {
      continue;
    }
    latestByInstance.set(parsed.instanceId, { ...parsed, lastSeen });
  }
  return [...latestByInstance.values()];
}

/** Drops instances whose last frame is older than the stale threshold. */
export function filterStaleInstances(
  instances: readonly InstanceLeadership[],
  now: number,
): InstanceLeadership[] {
  return instances.filter(
    (instance) => now - instance.lastSeen <= LEADERSHIP_STALE_MS,
  );
}

/**
 * The instance to surface as leader: the freshest (`max(lastSeen)`) among
 * those reporting `isLeader`. After a leader window crashes, the survivor's
 * `isLeader: true` and the dead window's stale `isLeader: true` coexist for up
 * to one stale window; picking the freshest converges to the survivor without
 * a "contested" UI state. Returns null when no instance currently leads.
 */
export function selectFreshestLeader(
  instances: readonly InstanceLeadership[],
): InstanceLeadership | null {
  let leader: InstanceLeadership | null = null;
  for (const instance of instances) {
    if (!instance.isLeader) {
      continue;
    }
    if (!leader || instance.lastSeen > leader.lastSeen) {
      leader = instance;
    }
  }
  return leader;
}
