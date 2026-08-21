import type {
  AgentMemoryListing,
  EngramEntry,
} from "@/shared/api/tauriEngrams";

/** A `mem/...` slug that another body referenced via `[[slug]]` but which
 * has no decrypted entry in the listing — either tombstoned or never written. */
export type DanglingRef = {
  /** The unresolved slug (e.g. `mem/values/honesty`). */
  slug: string;
  /** Slugs of the entries that referenced it. Helps the UI explain why
   *  a dangling ref is showing up. */
  referencedBy: string[];
};

/** A node in the reachability tree rooted at `core`. Children are themselves
 *  nodes — recursive rendering. The same engram never appears twice (visited
 *  set guards cycles); a ref to an already-visited node is dropped from the
 *  tree (the node still lives at its first appearance). */
export type MemoryTreeNode = {
  entry: EngramEntry;
  children: MemoryTreeNode[];
};

/** Output of {@link buildMemoryGraph}: the full panel-ready view. */
export type MemoryGraph = {
  /** Tree rooted at `core`. `null` when the agent has no core memory. */
  rootedTree: MemoryTreeNode | null;
  /** Memories not reachable from `core` via `[[slug]]` refs. Sorted by slug. */
  orphans: EngramEntry[];
  /** Refs in any decrypted body that didn't resolve to a known entry. */
  dangling: DanglingRef[];
};

/**
 * Build the panel's view of an {@link AgentMemoryListing}: tree rooted at
 * `core`, orphan list, and dangling-ref list.
 *
 * Why a single pass:
 * - Reachable vs orphans is derivable from the same BFS; no need to run
 *   twice.
 * - Dangling-ref detection drops out for free — any ref whose slug isn't
 *   in the slug→entry index is dangling, regardless of which body cited
 *   it.
 *
 * Cycle handling: a `visited` set is maintained per BFS. The first time a
 * memory is reached, it becomes a node in the tree; subsequent refs to it
 * are silently ignored. This means the *tree* is acyclic by construction
 * even though the underlying graph may have cycles.
 *
 * Tombstones never appear here — they're filtered out at the Rust layer
 * before this function ever sees them. A `[[slug]]` ref to a tombstoned
 * memory therefore shows up as a dangling ref, which is the right user-
 * facing signal ("this memory used to exist but doesn't anymore").
 */
export function buildMemoryGraph(listing: AgentMemoryListing): MemoryGraph {
  // slug → entry, used for both reachability and dangling-ref detection.
  // `core` lives in this index too so that a `[[core]]` ref from a memory
  // back-references correctly (rare but possible).
  const bySlug = new Map<string, EngramEntry>();
  if (listing.core) bySlug.set(listing.core.slug, listing.core);
  for (const m of listing.memories) bySlug.set(m.slug, m);

  // Track which entries we've already placed in the tree.
  const visited = new Set<string>();
  // refSlug → list of slugs that referenced it. Built lazily as we discover
  // refs that don't resolve.
  const danglingMap = new Map<string, string[]>();

  function recordDangling(refSlug: string, referencedBy: string): void {
    const list = danglingMap.get(refSlug);
    if (list) {
      // Don't double-count if the same body refs the same dangling slug
      // twice — duplicate `[[foo]] [[foo]]` should still surface a single
      // referrer in the UI.
      if (!list.includes(referencedBy)) list.push(referencedBy);
    } else {
      danglingMap.set(refSlug, [referencedBy]);
    }
  }

  function buildNode(entry: EngramEntry): MemoryTreeNode {
    visited.add(entry.slug);
    const children: MemoryTreeNode[] = [];
    for (const refSlug of entry.outgoingRefs) {
      const target = bySlug.get(refSlug);
      if (!target) {
        recordDangling(refSlug, entry.slug);
        continue;
      }
      // Skip self-refs and already-visited nodes; the visited guard makes
      // the recursion finite even on cyclic graphs.
      if (visited.has(target.slug)) continue;
      children.push(buildNode(target));
    }
    return { entry, children };
  }

  const rootedTree = listing.core ? buildNode(listing.core) : null;

  // Orphans = every memory not visited during the BFS from core. Sort by
  // slug so the UI is deterministic across refetches.
  const orphans = listing.memories
    .filter((m) => !visited.has(m.slug))
    .slice()
    .sort((a, b) => a.slug.localeCompare(b.slug));

  // Even orphans can themselves cite refs we should surface. Walk them
  // (no tree, just ref resolution) so dangling targets are complete.
  for (const o of orphans) {
    for (const refSlug of o.outgoingRefs) {
      if (!bySlug.has(refSlug)) recordDangling(refSlug, o.slug);
    }
  }

  const dangling: DanglingRef[] = Array.from(danglingMap.entries())
    .map(([slug, referencedBy]) => ({ slug, referencedBy }))
    .sort((a, b) => a.slug.localeCompare(b.slug));

  return { rootedTree, orphans, dangling };
}
