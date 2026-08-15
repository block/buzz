/**
 * Turn a flat list of channel files plus their `supersedes` links into version
 * chains: each newest file grouped with every older version behind it.
 *
 * Pure and dependency-free, in a `.mjs` sibling so `node:test` exercises the
 * exact source the UI runs (same rationale as `applyEditTagOverlay.mjs`). The
 * graph walking here is where malformed data turns into hangs or lost files,
 * so it needs tests that do not require mounting React.
 *
 * The input graph is user-generated and arrives over a relay, so it is not
 * assumed to be well-formed. Cycles (A supersedes B supersedes A), forks (two
 * files both claiming the same parent) and links pointing at files that were
 * deleted or never fetched are all possible, and every walk below is bounded
 * so that none of them can drop a file or spin.
 */

/**
 * Walk from `startId` to the newest version reachable from it.
 *
 * Returns the head's event id — `startId` itself when nothing supersedes it.
 * `seen` bounds the walk so a cycle terminates instead of looping forever;
 * on hitting one we stop at the last id we can vouch for rather than
 * pretending a head exists.
 */
export function resolveLatestEventId(startId, supersededByEventId) {
  let current = startId;
  const seen = new Set([current]);
  for (;;) {
    const next = supersededByEventId.get(current);
    if (!next || seen.has(next)) return current;
    seen.add(next);
    current = next;
  }
}

/**
 * Build the `older -> newer` index the walks need.
 *
 * A file's `supersedes` names the file it replaces, so the edge is inverted
 * here. Where two files claim the same parent (a fork — two people uploaded a
 * new version of the same document independently) the more recent upload wins
 * the edge, and the loser is left as its own chain head rather than being
 * silently swallowed: it is a real file someone shared, and hiding it under a
 * sibling it has no relationship to would be worse than showing both.
 */
function buildSupersededByIndex(files) {
  const byEventId = new Map(files.map((file) => [file.eventId, file]));
  const supersededByEventId = new Map();

  for (const file of files) {
    const olderId = file.supersedes;
    if (!olderId || olderId === file.eventId) continue;
    // A link to a file that is not in this list (deleted, or beyond the pages
    // we fetched) is dropped: we cannot group against something we cannot show.
    if (!byEventId.has(olderId)) continue;

    const incumbent = supersededByEventId.get(olderId);
    if (incumbent === undefined) {
      supersededByEventId.set(olderId, file.eventId);
      continue;
    }
    const incumbentFile = byEventId.get(incumbent);
    if ((file.uploadedAt ?? 0) > (incumbentFile?.uploadedAt ?? 0)) {
      supersededByEventId.set(olderId, file.eventId);
    }
  }

  return { byEventId, supersededByEventId };
}

/**
 * Group `files` into version chains.
 *
 * Returns one entry per chain: `{ latest, older }`, where `older` is every
 * prior version newest-first. Files with no version links come back as their
 * own chain with an empty `older`, so callers can render one uniform list
 * rather than special-casing unversioned files.
 *
 * Chains are ordered by their latest file's upload time, newest first, so the
 * caller's list ordering does not change just because a file gained a version.
 */
export function buildFileVersionChains(files) {
  const list = (files ?? []).filter((file) => file && file.eventId);
  const { byEventId, supersededByEventId } = buildSupersededByIndex(list);

  // A file is a chain head when nothing supersedes it.
  const heads = list.filter((file) => !supersededByEventId.has(file.eventId));

  const chains = [];
  const claimed = new Set();

  for (const head of heads) {
    const older = [];
    // Walk backwards from the head through `supersedes`, bounded by `seen` so
    // a cycle cannot loop and cannot re-add a file already in this chain.
    const seen = new Set([head.eventId]);
    let current = head;
    for (;;) {
      const olderId = current.supersedes;
      if (!olderId || seen.has(olderId)) break;
      const olderFile = byEventId.get(olderId);
      if (!olderFile) break;
      // Only follow the edge if it is the one we kept — otherwise a fork's
      // losing branch would be walked into two different chains.
      if (supersededByEventId.get(olderId) !== current.eventId) break;
      seen.add(olderId);
      older.push(olderFile);
      claimed.add(olderId);
      current = olderFile;
    }
    chains.push({ latest: head, older });
  }

  // A cycle has no head, so its files never appear above. Surface them as
  // standalone chains rather than dropping them: a malformed link must not
  // make somebody's file vanish from the Files tab.
  for (const file of list) {
    if (supersededByEventId.has(file.eventId) && !claimed.has(file.eventId)) {
      chains.push({ latest: file, older: [] });
    }
  }

  chains.sort(
    (a, b) => (b.latest.uploadedAt ?? 0) - (a.latest.uploadedAt ?? 0),
  );
  return chains;
}

/**
 * Map every file's event id to the event id of the newest version of it.
 *
 * This is what "view latest" navigates with: a file three versions back
 * resolves straight to the head rather than making the user click through each
 * intermediate step.
 */
export function buildLatestVersionIndex(files) {
  const list = (files ?? []).filter((file) => file && file.eventId);
  const { supersededByEventId } = buildSupersededByIndex(list);

  const latestByEventId = new Map();
  for (const file of list) {
    latestByEventId.set(
      file.eventId,
      resolveLatestEventId(file.eventId, supersededByEventId),
    );
  }
  return latestByEventId;
}
