import { useQuery } from "@tanstack/react-query";

import { invokeTauri } from "@/shared/api/tauri";
import { useFeatureEnabled } from "@/shared/features/useFeatureEnabled";

/**
 * Serialised shape of `~/Library/Application Support/xyz.block.buzz.app/roster/canonical-projection.json`
 * — mirror of the Rust `CanonicalProjectionConfig` returned by
 * `get_canonical_projection_config`. Only the fields consumed on the
 * frontend are typed; unrelated fields ride through as `unknown` so
 * schema evolution does not force a TS bump every time.
 *
 * Spec: `PLANS/CANONICAL_ROSTER_PROJECTION_SPEC_2026_08_22.md` §3.1.
 */
export type RawCanonicalProjectionConfig = {
  version: number;
  issued: string;
  revised: string;
  authorizing_events?: string[];
  canonical: Array<{ identity: string; host: string; pubkey: string }>;
  display_name_source_priority?: string[];
  display_name_overrides?: Record<string, string>;
};

const EMPTY_OVERRIDES: Readonly<Record<string, string>> = Object.freeze({});

const QUERY_KEY = ["canonicalRosterProjection", "config"] as const;

/**
 * Returns the pubkey → display-name overrides declared in the projection
 * config, or an empty map when the feature flag is off / the file is
 * absent. Never throws for a missing file; a parse error is surfaced by
 * react-query and yields the empty map so a bad config cannot hide
 * agents from the grid.
 *
 * Callers use this to preserve a canonical display name for a pubkey
 * whose signed kind-0 has been mutated by a legacy migration
 * (spec §3.4 / §5.3 — Bumble Air's `Pollen` label).
 */
export { pickProjectionOverriddenTitle } from "./pickProjectionOverriddenTitle";

export function useCanonicalProjectionOverrides(): Readonly<
  Record<string, string>
> {
  const enabled = useFeatureEnabled("canonicalRosterProjection");
  const { data } = useQuery({
    enabled,
    queryKey: QUERY_KEY,
    queryFn: async () =>
      invokeTauri<RawCanonicalProjectionConfig | null>(
        "get_canonical_projection_config",
      ),
    staleTime: 60_000,
  });
  if (!enabled || !data) return EMPTY_OVERRIDES;
  return data.display_name_overrides ?? EMPTY_OVERRIDES;
}
