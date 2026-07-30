import assert from "node:assert/strict";
import test from "node:test";

import {
  findLastOwnCorrectable,
  resolveSelfCorrection,
} from "./selfCorrectionEdit.ts";

const KIND_SYSTEM_MESSAGE = 40099;

const msg = (overrides) => ({
  id: "id",
  createdAt: 1,
  pubkey: "me",
  body: "",
  pending: false,
  tags: [],
  ...overrides,
});

// ── findLastOwnCorrectable ──────────────────────────────────────────────────

test("picks the author's most recent editable message", () => {
  const target = findLastOwnCorrectable(
    [
      msg({ id: "a", pubkey: "me", createdAt: 1 }),
      msg({ id: "b", pubkey: "me", createdAt: 3 }),
      msg({ id: "c", pubkey: "me", createdAt: 2 }),
    ],
    "me",
  );
  assert.equal(target?.id, "b");
});

test("ignores others' messages, system rows, and pending sends", () => {
  assert.equal(
    findLastOwnCorrectable(
      [
        msg({ id: "other", pubkey: "you", createdAt: 9 }),
        msg({
          id: "system",
          pubkey: "me",
          kind: KIND_SYSTEM_MESSAGE,
          createdAt: 8,
        }),
        msg({ id: "pending", pubkey: "me", pending: true, createdAt: 7 }),
        msg({ id: "mine", pubkey: "me", createdAt: 2 }),
      ],
      "me",
    )?.id,
    "mine",
  );
});

test("returns null when the author has no eligible message", () => {
  assert.equal(findLastOwnCorrectable([msg({ pubkey: "you" })], "me"), null);
});

// ── resolveSelfCorrection ───────────────────────────────────────────────────

test("resolves a plain-text correction to the target's edit", () => {
  const edit = resolveSelfCorrection(
    "s/hullo/hello/",
    [msg({ id: "m1", pubkey: "me", body: "hullo there" })],
    "me",
    [],
  );
  assert.deepEqual(edit, { eventId: "m1", content: "hello there", tags: [] });
});

test("returns null when the text is not a command", () => {
  assert.equal(
    resolveSelfCorrection(
      "just a normal message",
      [msg({ id: "m1", pubkey: "me", body: "hi" })],
      "me",
      [],
    ),
    null,
  );
});

test("returns null when there is no editable target", () => {
  assert.equal(
    resolveSelfCorrection("s/a/b/", [msg({ pubkey: "you" })], "me", []),
    null,
  );
});

test("returns null when the pattern is absent from the target", () => {
  assert.equal(
    resolveSelfCorrection(
      "s/xyz/q/",
      [msg({ id: "m1", pubkey: "me", body: "hello" })],
      "me",
      [],
    ),
    null,
  );
});

test("re-attaches NIP-30 emoji tags for shortcodes in the corrected body", () => {
  const edit = resolveSelfCorrection(
    "s/hi/yo/",
    [msg({ id: "m1", pubkey: "me", body: "hi :bee:" })],
    "me",
    [{ shortcode: "bee", url: "https://cdn/bee.png" }],
  );
  assert.equal(edit?.content, "yo :bee:");
  assert.ok(
    edit?.tags.some((tag) => tag[0] === "emoji" && tag[1] === "bee"),
    "expected an emoji tag for :bee:",
  );
});
