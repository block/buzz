import assert from "node:assert/strict";
import test from "node:test";

import {
  subtreeMaxCreatedAt,
  buildDirectReplyIdsByParentId,
  buildCreatedAtByMessageId,
  buildRepliesByRootId,
} from "./subtreeCreatedAt.ts";

// The thread-open read ceiling: opening a thread advances its read frontier to
// subtreeMaxCreatedAt(headId), the newest createdAt anywhere in the head's
// subtree. The thread-open caller starts the walk at the ROOT (consume the
// whole thread) and folds in rootId-matched orphans; the expand caller starts
// at a BRANCH node (consume only that branch) with no rootId folding. These
// tests pin both — Case 1's orphan-misses-ceiling defect and the call-site
// split that must NOT let expand cross into a sibling branch.

const msg = (id, parentId, createdAt, rootId) => ({
  id,
  parentId,
  rootId: rootId ?? parentId ?? id,
  createdAt,
});

// Branch-scoped ceiling — the expand caller. Parent-chain walk only.
const ceiling = (headId, messages) =>
  subtreeMaxCreatedAt(
    headId,
    buildDirectReplyIdsByParentId(messages),
    buildCreatedAtByMessageId(messages),
  );

// Root-scoped ceiling — the thread-open caller. Folds in replies that resolve
// to the head by rootId, so a severed orphan still raises the ceiling.
const rootCeiling = (headId, messages) =>
  subtreeMaxCreatedAt(
    headId,
    buildDirectReplyIdsByParentId(messages),
    buildCreatedAtByMessageId(messages),
    buildRepliesByRootId(messages),
  );

// (3) Case 1 — orphan misses the thread-open ceiling. The deep reply `c` is the
// newest content (300), but its middle ancestor `b` is unloaded, so `c` keys
// under absent "b" and the parentId-only walk never reaches it. The root-scoped
// ceiling folds in `c` by its rootId ("root", which travels with the event via
// getThreadReference), so it reaches 300 and the channel-root badge can clear.
test("openThreadCeiling_deepOrphanMissingAncestor_includedInCeiling", () => {
  const loaded = [
    msg("root", null, 50, "root"),
    msg("a", "root", 200, "root"),
    // msg("b", "a", ...) — intentionally absent: unloaded middle ancestor.
    msg("c", "b", 300, "root"),
  ];
  assert.equal(rootCeiling("root", loaded), 300);
});

// (3) control — with the middle ancestor present the chain is intact and the
// root-started walk already reaches `c`, so the ceiling is 300 today.
test("openThreadCeiling_fullChain_reachesDeepest", () => {
  const loaded = [
    msg("root", null, 50, "root"),
    msg("a", "root", 200, "root"),
    msg("b", "a", 250, "root"),
    msg("c", "b", 300, "root"),
  ];
  assert.equal(ceiling("root", loaded), 300);
});

// (4) Expand does NOT cross siblings — the call-site split. Expanding branch
// `a` starts the ceiling walk at `a`, so it consumes only `a`'s subtree
// (newest = a2 at 220) and must NOT advance past sibling branch `d`'s unread
// reply (`d1` at 400). If expand keyed the ceiling on the ROOT, it would jump
// to 400 and silently consume the sibling — the defect Thufir flagged.
//
// GREEN on current code: subtreeMaxCreatedAt is branch-scoped by construction
// when started at the branch node. This test LOCKS that property so the
// redesign's rootId re-key cannot accidentally make expand root-scoped.
test("openThreadCeiling_expandBranch_doesNotCrossSibling", () => {
  const loaded = [
    msg("root", null, 50, "root"),
    msg("a", "root", 100, "root"),
    msg("a1", "a", 210, "root"),
    msg("a2", "a", 220, "root"),
    msg("d", "root", 120, "root"),
    msg("d1", "d", 400, "root"), // sibling branch's newer unread reply
  ];
  // Expanding branch `a` reaches only a/a1/a2 — ceiling is 220, NOT 400.
  assert.equal(ceiling("a", loaded), 220);
  // The root-started ceiling DOES span everything, 400 — proving the two
  // call-sites are genuinely different scopes, not the same value by accident.
  assert.equal(ceiling("root", loaded), 400);
});

// (4) companion — expanding a branch returns just the branch head's own
// createdAt when the branch has no replies, never reaching across to siblings.
test("openThreadCeiling_expandLeafBranch_ownCreatedAtOnly", () => {
  const loaded = [
    msg("root", null, 50, "root"),
    msg("a", "root", 100, "root"),
    msg("d", "root", 120, "root"),
    msg("d1", "d", 400, "root"),
  ];
  // Branch `a` is a leaf: ceiling is its own 100, unaffected by sibling d1@400.
  assert.equal(ceiling("a", loaded), 100);
});
