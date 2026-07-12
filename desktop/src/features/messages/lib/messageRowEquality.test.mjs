import assert from "node:assert/strict";
import test from "node:test";

import {
  depthGuideActionsEqual,
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
