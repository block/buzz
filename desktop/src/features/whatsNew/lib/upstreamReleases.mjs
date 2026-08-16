/**
 * Turn GitHub's releases payload for upstream `block/buzz` into history rows,
 * and interleave them with this fork's own releases by date.
 *
 * Pure and dependency-free, in a `.mjs` sibling so `node:test` exercises the
 * exact source the UI runs (same rationale as `applyEditTagOverlay.mjs`). The
 * input is a third-party API response, so nothing about its shape is assumed:
 * every field is checked before use and anything unrecognised is dropped
 * rather than rendered.
 *
 * Why filter at all: upstream keeps releasing past the version this fork last
 * caught up to, and those releases describe features this build does not have.
 * Showing them would turn the history into a list of things that appear
 * broken. The ceiling is the fork's own core version, so the list grows only
 * when a catch-up merge actually lands the work.
 */

/** Upstream's desktop releases are tagged `desktop-v<semver>`. */
const DESKTOP_TAG_PREFIX = "desktop-v";

/**
 * Parse a dotted numeric version into comparable parts.
 *
 * Returns null for anything that is not at least one number, which is the
 * signal to drop the release rather than guess where it belongs.
 */
function parseVersionParts(version) {
  const core = String(version ?? "")
    .trim()
    .split("-")[0];
  if (!/^\d+(\.\d+)*$/.test(core)) return null;
  return core.split(".").map(Number);
}

/**
 * Compare two dotted versions. Negative when `a` is older.
 *
 * Missing trailing segments count as 0, so `0.5` and `0.5.0` are equal.
 */
export function compareVersions(a, b) {
  const left = parseVersionParts(a);
  const right = parseVersionParts(b);
  if (!left || !right) return 0;
  const length = Math.max(left.length, right.length);
  for (let i = 0; i < length; i += 1) {
    const diff = (left[i] ?? 0) - (right[i] ?? 0);
    if (diff !== 0) return diff;
  }
  return 0;
}

/**
 * The upstream version this fork currently carries.
 *
 * The fork's version is `<upstream core>-<k2alpha counter>`, so everything
 * before the dash is exactly what upstream release the code came from.
 */
export function coreVersionOf(forkVersion) {
  const core = String(forkVersion ?? "")
    .trim()
    .split("-")[0];
  return /^\d+(\.\d+)*$/.test(core) ? core : null;
}

/**
 * Normalise GitHub's releases payload into history rows.
 *
 * Drops drafts (not published), prereleases (not what anyone is running),
 * non-desktop tags (upstream also ships `relay-v*` and `chart-v*` on the same
 * repo), and anything at or below the ceiling check in `atOrBelowVersion`.
 */
export function parseUpstreamReleases(payload, atOrBelowVersion) {
  if (!Array.isArray(payload)) return [];

  const rows = [];
  for (const release of payload) {
    if (!release || typeof release !== "object") continue;
    if (release.draft === true || release.prerelease === true) continue;

    const tag = typeof release.tag_name === "string" ? release.tag_name : "";
    if (!tag.startsWith(DESKTOP_TAG_PREFIX)) continue;

    const version = tag.slice(DESKTOP_TAG_PREFIX.length);
    if (!parseVersionParts(version)) continue;

    // The ceiling is what keeps unmerged upstream work out of the list.
    if (atOrBelowVersion && compareVersions(version, atOrBelowVersion) > 0) {
      continue;
    }

    const published =
      typeof release.published_at === "string" ? release.published_at : null;

    rows.push({
      key: `upstream:${tag}`,
      source: "upstream",
      version,
      date: published ? published.slice(0, 10) : null,
      url: typeof release.html_url === "string" ? release.html_url : null,
    });
  }
  return rows;
}

/** This fork's own entries, in the same shape as the upstream rows. */
export function localReleaseRows(entries) {
  return (entries ?? [])
    .filter((entry) => entry && entry.version)
    .map((entry) => ({
      key: `local:${entry.version}`,
      source: "local",
      version: entry.version,
      date: entry.date ?? null,
      bullets: entry.bullets ?? [],
    }));
}

/**
 * One timeline, newest first.
 *
 * Undated rows sort to the end rather than to the top: a missing date means
 * unknown, and floating an unknown release above dated ones would misstate
 * the history. In practice the only undated entries are the two oldest, so
 * last is also correct.
 *
 * Ties break local-before-upstream, then by version descending, so a fork
 * release and the upstream release it caught up to sit in a stable order
 * instead of swapping between renders.
 */
export function mergeReleaseTimeline(localRows, upstreamRows) {
  const rows = [...(localRows ?? []), ...(upstreamRows ?? [])];

  rows.sort((a, b) => {
    if (a.date && b.date) {
      if (a.date !== b.date) return a.date < b.date ? 1 : -1;
    } else if (a.date !== b.date) {
      return a.date ? -1 : 1;
    }
    if (a.source !== b.source) return a.source === "local" ? -1 : 1;
    return compareVersions(b.version, a.version);
  });

  return rows;
}
