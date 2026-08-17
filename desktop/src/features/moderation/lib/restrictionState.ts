/**
 * Derive a member's live moderation state from a `CommunityRestriction` row.
 *
 * Shared by the two admin surfaces that gate on it (the members sidebar and the
 * per-message action cluster) so the ban/timeout reads can't drift.
 */

/** Whether a member is currently banned and/or timed out. */
export type MemberRestrictionState = {
  banned: boolean;
  timedOut: boolean;
};

/**
 * Coerce a restriction timestamp to epoch milliseconds. The wire emits
 * `DateTime<Utc>` as an RFC3339 string, but the shared type still tolerates the
 * legacy `number` (unix seconds) shape, so handle both: strings parse as ISO,
 * numbers are treated as unix seconds. Returns `null` for absent or unparseable
 * values — fails closed, so a bad value never renders a phantom timeout.
 */
export function parseRestrictionTimestampMs(
  value: string | number | null,
): number | null {
  if (value == null) return null;
  if (typeof value === "number") return value * 1000;
  const parsed = Date.parse(value);
  return Number.isNaN(parsed) ? null : parsed;
}

/**
 * True when a `mutedUntil` value is still in the future relative to `nowMs`.
 * An absent or unparseable value is *not* an active timeout (fail closed to
 * "not timed out" so the UI doesn't strand a member who has no live mute).
 */
export function isTimedOut(
  mutedUntil: string | number | null,
  nowMs: number = Date.now(),
): boolean {
  const ms = parseRestrictionTimestampMs(mutedUntil);
  return ms != null && ms > nowMs;
}
