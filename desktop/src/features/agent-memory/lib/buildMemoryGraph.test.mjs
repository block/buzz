import assert from "node:assert/strict";
import test from "node:test";

import { buildMemoryGraph } from "./buildMemoryGraph.ts";

function entry(slug, body, outgoingRefs = []) {
  return {
    slug,
    body,
    eventId: `id-${slug}`,
    createdAt: 0,
    outgoingRefs,
  };
}

test("buildMemoryGraph: empty listing → null tree, no orphans, no dangling", () => {
  const g = buildMemoryGraph({
    core: null,
    memories: [],
    truncated: false,
    fetchedAt: 0,
  });
  assert.equal(g.rootedTree, null);
  assert.deepEqual(g.orphans, []);
  assert.deepEqual(g.dangling, []);
});

test("buildMemoryGraph: core only, no refs → tree of one, no orphans", () => {
  const g = buildMemoryGraph({
    core: entry("core", "I am Bart"),
    memories: [],
    truncated: false,
    fetchedAt: 0,
  });
  assert.equal(g.rootedTree?.entry.slug, "core");
  assert.deepEqual(g.rootedTree?.children, []);
  assert.deepEqual(g.orphans, []);
});

test("buildMemoryGraph: core → mem/foo → mem/bar reachable chain", () => {
  const g = buildMemoryGraph({
    core: entry("core", "see [[mem/foo]]", ["mem/foo"]),
    memories: [
      entry("mem/foo", "see [[mem/bar]]", ["mem/bar"]),
      entry("mem/bar", "leaf", []),
    ],
    truncated: false,
    fetchedAt: 0,
  });
  assert.equal(g.rootedTree?.entry.slug, "core");
  assert.equal(g.rootedTree?.children.length, 1);
  assert.equal(g.rootedTree?.children[0].entry.slug, "mem/foo");
  assert.equal(g.rootedTree?.children[0].children[0].entry.slug, "mem/bar");
  assert.deepEqual(g.orphans, []);
  assert.deepEqual(g.dangling, []);
});

test("buildMemoryGraph: orphans surface when no path from core", () => {
  const g = buildMemoryGraph({
    core: entry("core", "isolated"),
    memories: [entry("mem/zeta", "z", []), entry("mem/alpha", "a", [])],
    truncated: false,
    fetchedAt: 0,
  });
  // Orphans sorted by slug → alpha before zeta
  assert.deepEqual(
    g.orphans.map((o) => o.slug),
    ["mem/alpha", "mem/zeta"],
  );
});

test("buildMemoryGraph: cycle is broken — no infinite recursion", () => {
  const g = buildMemoryGraph({
    core: entry("core", "see [[mem/a]]", ["mem/a"]),
    memories: [
      entry("mem/a", "see [[mem/b]]", ["mem/b"]),
      entry("mem/b", "see [[mem/a]]", ["mem/a"]), // cycle back to a
    ],
    truncated: false,
    fetchedAt: 0,
  });
  assert.equal(g.rootedTree?.entry.slug, "core");
  const a = g.rootedTree?.children[0];
  assert.equal(a?.entry.slug, "mem/a");
  const b = a?.children[0];
  assert.equal(b?.entry.slug, "mem/b");
  // The cycle back to `a` is dropped — visited set guards it.
  assert.deepEqual(b?.children, []);
  assert.deepEqual(g.orphans, []);
});

test("buildMemoryGraph: dangling refs surface with referrers", () => {
  const g = buildMemoryGraph({
    core: entry("core", "see [[mem/missing]]", ["mem/missing"]),
    memories: [
      entry("mem/orphan", "see [[mem/also-missing]]", ["mem/also-missing"]),
    ],
    truncated: false,
    fetchedAt: 0,
  });
  // mem/missing referenced by core; mem/also-missing referenced by mem/orphan
  assert.deepEqual(g.dangling, [
    { slug: "mem/also-missing", referencedBy: ["mem/orphan"] },
    { slug: "mem/missing", referencedBy: ["core"] },
  ]);
});

test("buildMemoryGraph: duplicate refs from same body deduplicate referrer list", () => {
  const g = buildMemoryGraph({
    core: entry("core", "see [[mem/missing]] and [[mem/missing]]", [
      "mem/missing",
      "mem/missing",
    ]),
    memories: [],
    truncated: false,
    fetchedAt: 0,
  });
  assert.deepEqual(g.dangling, [
    { slug: "mem/missing", referencedBy: ["core"] },
  ]);
});

test("buildMemoryGraph: shared child appears once at first encounter", () => {
  // core → mem/a → mem/shared
  // core → mem/b → mem/shared
  // mem/shared should attach under mem/a (DFS first), not mem/b.
  const g = buildMemoryGraph({
    core: entry("core", "", ["mem/a", "mem/b"]),
    memories: [
      entry("mem/a", "", ["mem/shared"]),
      entry("mem/b", "", ["mem/shared"]),
      entry("mem/shared", "", []),
    ],
    truncated: false,
    fetchedAt: 0,
  });
  const root = g.rootedTree;
  assert.equal(root?.children[0].entry.slug, "mem/a");
  assert.equal(root?.children[0].children[0].entry.slug, "mem/shared");
  assert.equal(root?.children[1].entry.slug, "mem/b");
  // mem/b loses the duplicate child because `shared` was already visited.
  assert.deepEqual(root?.children[1].children, []);
  assert.deepEqual(g.orphans, []);
});
