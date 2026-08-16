import assert from "node:assert/strict";
import test from "node:test";

import {
  depthGuideActionsEqual,
  requestStatusEqual,
  numberArrayEqual,
  reactionsEqual,
  tagsEqual,
} from "./messageRowEquality.ts";

// These helpers exist so MessageRow's memo holds when arrays are rebuilt
// with fresh identities but unchanged values (every ingest/refetch does
// this). Equal-by-value must return true; any real change must return false.

test("tagsEqual: fresh identity, same values → equal", () => {
  assert.equal(
    tagsEqual(
      [
        ["e", "abc"],
        ["h", "chan"],
      ],
      [
        ["e", "abc"],
        ["h", "chan"],
      ],
    ),
    true,
  );
});

test("tagsEqual: changed tag value → not equal", () => {
  assert.equal(tagsEqual([["e", "abc"]], [["e", "abd"]]), false);
  assert.equal(
    tagsEqual(
      [["e", "abc"]],
      [
        ["e", "abc"],
        ["p", "x"],
      ],
    ),
    false,
  );
  assert.equal(tagsEqual(undefined, [["e", "abc"]]), false);
  assert.equal(tagsEqual(undefined, undefined), true);
});

test("reactionsEqual: fresh identity, same values → equal", () => {
  const make = () => [
    {
      emoji: "🔥",
      count: 2,
      reactedByCurrentUser: true,
      users: [
        { pubkey: "aa", displayName: "A", avatarUrl: null },
        { pubkey: "bb", displayName: "B", avatarUrl: "u" },
      ],
    },
  ];
  assert.equal(reactionsEqual(make(), make()), true);
});

test("reactionsEqual: any changed field → not equal", () => {
  const base = {
    emoji: "🔥",
    count: 2,
    reactedByCurrentUser: false,
    users: [{ pubkey: "aa", displayName: "A", avatarUrl: null }],
  };
  assert.equal(reactionsEqual([base], [{ ...base, count: 3 }]), false);
  assert.equal(
    reactionsEqual([base], [{ ...base, reactedByCurrentUser: true }]),
    false,
  );
  assert.equal(
    reactionsEqual(
      [base],
      [
        {
          ...base,
          users: [{ pubkey: "cc", displayName: "A", avatarUrl: null }],
        },
      ],
    ),
    false,
  );
  assert.equal(reactionsEqual(undefined, undefined), true);
  assert.equal(reactionsEqual([base], undefined), false);
});

test("numberArrayEqual", () => {
  assert.equal(numberArrayEqual([1, 2], [1, 2]), true);
  assert.equal(numberArrayEqual([1, 2], [2, 1]), false);
  assert.equal(numberArrayEqual(undefined, undefined), true);
  assert.equal(numberArrayEqual([1], undefined), false);
});

test("depthGuideActionsEqual: same values (message by id) → equal", () => {
  const message = { id: "m1" };
  const other = { id: "m1" };
  assert.equal(
    depthGuideActionsEqual(
      [{ active: false, depth: 1, label: "Collapse replies", message }],
      [{ active: false, depth: 1, label: "Collapse replies", message: other }],
    ),
    true,
  );
  assert.equal(
    depthGuideActionsEqual(
      [{ active: false, depth: 1, label: "Collapse replies", message }],
      [{ active: true, depth: 1, label: "Collapse replies", message }],
    ),
    false,
  );
});

test("requestStatusEqual: fresh identity, same values → equal", () => {
  const make = () => ({
    kind: "valid",
    outcome: { kind: "settled", state: "refused" },
    latestObservation: "refused",
    reason: "outside my delegation",
    warnings: [],
    resolved: true,
  });
  assert.equal(requestStatusEqual(make(), make()), true);
});

test("requestStatusEqual: changed field → not equal", () => {
  const base = {
    kind: "valid",
    outcome: { kind: "settled", state: "completed" },
    latestObservation: "completed",
    reason: "",
    warnings: [],
    resolved: true,
  };
  assert.equal(
    requestStatusEqual(base, {
      ...base,
      outcome: { kind: "settled", state: "refused" },
    }),
    false,
  );
  assert.equal(requestStatusEqual(base, { ...base, reason: "x" }), false);
  assert.equal(
    requestStatusEqual(base, { ...base, warnings: ["duplicate_terminal"] }),
    false,
  );
  assert.equal(
    requestStatusEqual(base, { ...base, latestObservation: "errored" }),
    false,
    "the raw latest observation is part of what the row renders",
  );
  // Same outcome kind, different state: the comparator must look past `kind`.
  assert.equal(
    requestStatusEqual(
      { ...base, outcome: { kind: "open", state: "responded" } },
      { ...base, outcome: { kind: "open", state: "errored" } },
    ),
    false,
  );
});

test("requestStatusEqual: classification kinds are compared", () => {
  const invalid = { kind: "invalid", reason: "missing_agent_target" };
  assert.equal(requestStatusEqual(invalid, { ...invalid }), true);
  assert.equal(
    requestStatusEqual(invalid, { kind: "invalid", reason: "missing_channel" }),
    false,
  );
  assert.equal(
    requestStatusEqual(invalid, {
      kind: "unsupported",
      reason: "multiple_agent_targets",
    }),
    false,
  );
});

test("requestStatusEqual: undefined handling", () => {
  assert.equal(requestStatusEqual(undefined, undefined), true);
  const base = {
    kind: "valid",
    outcome: { kind: "settled", state: "completed" },
    latestObservation: "completed",
    reason: "",
    warnings: [],
    resolved: true,
  };
  assert.equal(requestStatusEqual(base, undefined), false);
  assert.equal(requestStatusEqual(undefined, base), false);
});
