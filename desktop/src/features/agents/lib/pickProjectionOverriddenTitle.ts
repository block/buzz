/**
 * Pure display-name resolver for the canonical roster projection.
 * Returns the projection override when one exists for `agentPubkey`,
 * otherwise the fallback (persona display name or raw row name).
 *
 * Extracted from the hook so the precedence is testable without a React
 * host or import-alias resolver — the surrounding hook is a thin
 * react-query wrapper whose behavior reduces to this one line.
 *
 * Spec: `PLANS/CANONICAL_ROSTER_PROJECTION_SPEC_2026_08_22.md`
 * §3.4 / §6.5.
 */
export function pickProjectionOverriddenTitle(
  overrides: Readonly<Record<string, string>>,
  agentPubkey: string | undefined,
  fallback: string,
): string {
  if (!agentPubkey) return fallback;
  const override = overrides[agentPubkey];
  return override && override.length > 0 ? override : fallback;
}
