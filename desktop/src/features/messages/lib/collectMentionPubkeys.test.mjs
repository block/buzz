import assert from "node:assert/strict";
import test from "node:test";

import { collectMentionPubkeys } from "./collectMentionPubkeys.ts";

test("explicit mention map wins over same-named channel members", () => {
  const map = new Map([["Atlas", "pk-local"]]);
  const candidates = [
    {
      kind: "identity",
      pubkey: "pk-foreign",
      displayName: "Atlas",
      isMember: true,
      isAgent: true,
    },
    {
      kind: "identity",
      pubkey: "pk-local",
      displayName: "Atlas",
      isMember: true,
      isAgent: true,
      isManagedAgent: true,
    },
  ];

  assert.deepEqual(collectMentionPubkeys("@Atlas hi", map, candidates), [
    "pk-local",
  ]);
});

test("same-named members collapse to one pubkey, preferring managed", () => {
  const candidates = [
    {
      kind: "identity",
      pubkey: "pk-foreign",
      displayName: "Atlas",
      isMember: true,
      isAgent: true,
    },
    {
      kind: "identity",
      pubkey: "pk-local",
      displayName: "Atlas",
      isMember: true,
      isAgent: true,
      isManagedAgent: true,
    },
  ];

  assert.deepEqual(collectMentionPubkeys("@Atlas hi", new Map(), candidates), [
    "pk-local",
  ]);
});

test("distinct display names still resolve independently", () => {
  const candidates = [
    {
      kind: "identity",
      pubkey: "pk-a",
      displayName: "Alice",
      isMember: true,
      isAgent: false,
    },
    {
      kind: "identity",
      pubkey: "pk-b",
      displayName: "Bob",
      isMember: true,
      isAgent: true,
    },
  ];

  assert.deepEqual(
    collectMentionPubkeys("hey @Alice and @Bob", new Map(), candidates).sort(),
    ["pk-a", "pk-b"],
  );
});
