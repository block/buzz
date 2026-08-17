import type { ChangelogEntry } from "@/features/whatsNew/changelog";

export type ReleaseTimelineRow = {
  key: string;
  source: "local" | "upstream";
  version: string;
  /** `YYYY-MM-DD`, or null when unknown. */
  date: string | null;
  /** Present on local rows only. */
  bullets?: string[];
  /** Present on upstream rows only. */
  url?: string | null;
  /**
   * Position in `WHATS_NEW_CHANGELOG` (oldest-first). Local rows only.
   * The tiebreak for two releases on the same day — version strings cannot
   * serve, since `compareVersions` strips the prerelease suffix.
   */
  order?: number;
};

export function compareVersions(a: string, b: string): number;

export function coreVersionOf(forkVersion: string): string | null;

export function parseUpstreamReleases(
  payload: unknown,
  atOrBelowVersion: string | null,
): ReleaseTimelineRow[];

export function localReleaseRows(
  entries: readonly ChangelogEntry[] | undefined,
): ReleaseTimelineRow[];

export function mergeReleaseTimeline(
  localRows: readonly ReleaseTimelineRow[] | undefined,
  upstreamRows: readonly ReleaseTimelineRow[] | undefined,
): ReleaseTimelineRow[];
