import assert from "node:assert/strict";
import test from "node:test";
import { npubEncode } from "nostr-tools/nip19";

import {
  extractNip27MentionPubkeys,
  replaceNip27MentionsForDisplay,
} from "./nip27Mentions.ts";

const ALICE =
  "3bf0c63fcb93463407afdbb8b86e4e2c2e4e2c2e4e2c2e4e2c2e4e2c2e4e2c2e";
const aliceUri = `nostr:${npubEncode(ALICE)}`;

test("extractNip27MentionPubkeys: finds npub in prose", () => {
  const out = extractNip27MentionPubkeys(`hello ${aliceUri} world`);
  assert.deepEqual(out, [ALICE.toLowerCase()]);
});

test("extractNip27MentionPubkeys: skips code spans", () => {
  const out = extractNip27MentionPubkeys(`\`${aliceUri}\` then ${aliceUri}`);
  assert.deepEqual(out, [ALICE.toLowerCase()]);
});

test("replaceNip27MentionsForDisplay: maps known pubkeys to @name", () => {
  const out = replaceNip27MentionsForDisplay(`hi ${aliceUri}`, {
    Alice: ALICE,
  });
  assert.equal(out, "hi @Alice");
});

test("replaceNip27MentionsForDisplay: leaves unknown URIs", () => {
  const out = replaceNip27MentionsForDisplay(`hi ${aliceUri}`, {});
  assert.equal(out, `hi ${aliceUri}`);
});
