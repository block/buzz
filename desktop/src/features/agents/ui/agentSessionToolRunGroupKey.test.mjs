import assert from "node:assert/strict";
import test from "node:test";

import {
  getToolRunGroupKey,
  getToolRunGroupMemberKeys,
  resolveToolRunGroupIdentities,
  scopeToolRunGroupKey,
} from "./agentSessionToolRunGroupKey.ts";

const timestamp = "2026-06-14T22:20:23.000Z";

function tool(id, turnId = "turn-1") {
  return {
    id,
    type: "tool",
    renderClass: "shell",
    descriptor: { renderClass: "shell", label: "Ran command", preview: null },
    title: "Ran command",
    toolName: "shell",
    buzzToolName: null,
    status: "completed",
    args: {},
    result: "",
    isError: false,
    timestamp,
    startedAt: timestamp,
    completedAt: timestamp,
    turnId,
  };
}

test("the key is derived from turn and first leaf, not the batch id", () => {
  const items = [tool("tool:ch1:call-1"), tool("tool:ch1:call-2")];
  assert.equal(
    getToolRunGroupKey({ id: "summary:shell:tool:ch1:call-1", items }),
    "group:turn-1:tool:ch1:call-1",
  );
});

test("the key survives the same-kind to mixed transition", () => {
  const items = [tool("tool:ch1:call-1"), tool("tool:ch1:call-2")];
  const sameKind = getToolRunGroupKey({
    id: "summary:shell:tool:ch1:call-1",
    items,
  });
  const mixed = getToolRunGroupKey({
    id: "summary:mixed:tool:ch1:call-1",
    items: [...items, tool("tool:ch1:call-3")],
  });
  assert.equal(sameKind, mixed);
});

test("appending live calls to a growing group does not churn the key", () => {
  const first = tool("tool:ch1:call-1");
  const growing = getToolRunGroupKey({ id: "summary:mixed:x", items: [first] });
  const grown = getToolRunGroupKey({
    id: "summary:mixed:x",
    items: [first, tool("tool:ch1:call-2"), tool("tool:ch1:call-3")],
  });
  assert.equal(growing, grown);
});

test("two turns doing identical work never collide", () => {
  const a = getToolRunGroupKey({
    id: "summary:shell:a",
    items: [tool("tool:ch1:call-1", "turn-1")],
  });
  const b = getToolRunGroupKey({
    id: "summary:shell:a",
    items: [tool("tool:ch1:call-1", "turn-2")],
  });
  assert.notEqual(a, b);
});

test("a group without turn identity still keys off its first leaf", () => {
  assert.equal(
    getToolRunGroupKey({
      id: "summary:shell:x",
      items: [tool("tool:ch1:call-9", null)],
    }),
    "group:no-turn:tool:ch1:call-9",
  );
});

test("distinct groups in one turn get distinct keys", () => {
  const a = getToolRunGroupKey({
    id: "summary:shell:a",
    items: [tool("tool:ch1:call-1")],
  });
  const b = getToolRunGroupKey({
    id: "summary:shell:b",
    items: [tool("tool:ch1:call-7")],
  });
  assert.notEqual(a, b);
});

test("an empty group falls back to its batch id rather than colliding", () => {
  const a = getToolRunGroupKey({ id: "summary:mixed:a", items: [] });
  const b = getToolRunGroupKey({ id: "summary:mixed:b", items: [] });
  assert.notEqual(a, b);
});

// --- durable identity resolution --------------------------------------------

const noTable = new Map();

/** Resolve one pass and return `[keys, table]`. */
function resolve(groups, previous = noTable) {
  const { keys, table } = resolveToolRunGroupIdentities(groups, previous);
  return [keys, table];
}

test("member keys cover every leaf, not just the first", () => {
  assert.deepEqual(
    getToolRunGroupMemberKeys({
      id: "summary:shell:x",
      items: [tool("tool:ch1:call-1"), tool("tool:ch1:call-2")],
    }),
    ["group:turn-1:tool:ch1:call-1", "group:turn-1:tool:ch1:call-2"],
  );
});

test("reader state keys are isolated per transcript scope", () => {
  const groupKey = "group:turn-1:tool:channel:call-1";
  const fullViewer = scopeToolRunGroupKey("agent:channel:default", groupKey);
  const compactPreview = scopeToolRunGroupKey(
    "agent:channel:compactPreview",
    groupKey,
  );
  assert.notEqual(fullViewer, compactPreview);
  assert.match(fullViewer, /group:turn-1:tool:channel:call-1$/);
});

test("scope boundaries cannot collide", () => {
  assert.notEqual(
    scopeToolRunGroupKey("a:b", "c"),
    scopeToolRunGroupKey("a", "b:c"),
  );
});

test("a first pass anchors each group on its own first leaf", () => {
  const [keys] = resolve([
    ["group:t:a", "group:t:b"],
    ["group:t:x", "group:t:y"],
  ]);
  assert.deepEqual(keys, ["group:t:a", "group:t:x"]);
});

test("ejecting the leading leaf keeps the group's identity", () => {
  // The Finding B case: the first call fails, grouping ejects it, and the
  // remaining run must still answer to the identity the reader's state is under.
  const [, table] = resolve([["group:t:a", "group:t:b", "group:t:c"]]);
  const [keys] = resolve([["group:t:b", "group:t:c"]], table);
  assert.deepEqual(keys, ["group:t:a"]);
});

test("identity survives repeated ejections down to the last leaf", () => {
  let table = resolve([["group:t:a", "group:t:b", "group:t:c"]])[1];
  for (const remaining of [["group:t:b", "group:t:c"], ["group:t:c"]]) {
    const [keys, next] = resolve([remaining], table);
    assert.deepEqual(keys, ["group:t:a"]);
    table = next;
  }
});

test("a group that grows by appending keeps its identity", () => {
  const [, table] = resolve([["group:t:a", "group:t:b"]]);
  const [keys] = resolve([["group:t:a", "group:t:b", "group:t:c"]], table);
  assert.deepEqual(keys, ["group:t:a"]);
});

test("re-resolving an unchanged pass is idempotent", () => {
  const groups = [["group:t:a", "group:t:b"], ["group:t:x"]];
  const [first, table] = resolve(groups);
  const [second] = resolve(groups, table);
  assert.deepEqual(second, first);
});

test("a split run gives the identity to the part holding the earlier leaves", () => {
  // A failure in the MIDDLE splits one group into two. Both halves recognise
  // leaves from the original, but only one may inherit its reader state.
  const [, table] = resolve([
    ["group:t:a", "group:t:b", "group:t:c", "group:t:d"],
  ]);
  const [keys] = resolve([["group:t:a", "group:t:b"], ["group:t:d"]], table);
  assert.deepEqual(keys, ["group:t:a", "group:t:d"]);
});

test("a trailing split half never inherits the leading half's identity", () => {
  // Same split, but the ejected failure is the FIRST leaf as well, so the
  // leading half no longer holds the original anchor. It still claims it, and
  // the trailing half must not.
  const [, table] = resolve([
    ["group:t:a", "group:t:b", "group:t:c", "group:t:d"],
  ]);
  const [keys] = resolve([["group:t:b"], ["group:t:d"]], table);
  assert.deepEqual(keys, ["group:t:a", "group:t:d"]);
  assert.notEqual(keys[0], keys[1]);
});

test("two groups never share one identity", () => {
  const [, table] = resolve([["group:t:a", "group:t:b"]]);
  const [keys] = resolve([["group:t:a"], ["group:t:b"]], table);
  assert.equal(new Set(keys).size, 2);
});

test("wholly new groups do not inherit anything", () => {
  const [, table] = resolve([["group:t:a"]]);
  const [keys] = resolve([["group:t:z"]], table);
  assert.deepEqual(keys, ["group:t:z"]);
});

test("the remembered table drops leaves that left the transcript", () => {
  // Otherwise the table would grow for the lifetime of the community: every
  // tool call ever grouped would stay resident.
  const [, table] = resolve([["group:t:a", "group:t:b"]]);
  const [, next] = resolve([["group:t:b"]], table);
  assert.deepEqual([...next.keys()], ["group:t:b"]);
  assert.equal(next.get("group:t:b"), "group:t:a");
});

test("resolution never mutates the table it was given", () => {
  const [, table] = resolve([["group:t:a", "group:t:b"]]);
  const before = [...table.entries()].sort();
  resolve([["group:t:b"]], table);
  assert.deepEqual([...table.entries()].sort(), before);
});

test("a group whose own anchor is already claimed takes its next free leaf", () => {
  // Defensive, and asserted here at the function boundary because it is the
  // one collision the caller cannot rule out: a remembered identity can equal
  // some LATER group's own first leaf. Sharing one identity would mean two
  // cards sharing one reader state, so the loser falls through instead.
  const table = new Map([["group:t:a", "group:t:b"]]);
  const [keys] = resolve([["group:t:a"], ["group:t:b", "group:t:c"]], table);
  assert.deepEqual(keys, ["group:t:b", "group:t:c"]);
  assert.equal(new Set(keys).size, 2);
});
