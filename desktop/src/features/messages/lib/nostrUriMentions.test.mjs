import assert from "node:assert/strict";
import test from "node:test";

import { nip19 } from "nostr-tools";

import { extractNostrUriPubkeys } from "./nostrUriMentions.ts";

const SAVJETNIK =
  "cf72c7aaa9a829fa25371e86c9564acaaff1021595cad253158e4298bf3e828b";
const PI314 =
  "f3856e55ce82261142b9854e6450eabcee5e140617298ca51efb75ac9e14f0d9";

const savjetnikNpub = nip19.npubEncode(SAVJETNIK);
const pi314Npub = nip19.npubEncode(PI314);

test("extracts the pubkey from a nostr:npub URI", () => {
  assert.deepEqual(
    extractNostrUriPubkeys(`nostr:${savjetnikNpub} koliki je promet danas?`),
    [SAVJETNIK],
  );
});

test("extracts the pubkey from a nostr:nprofile URI", () => {
  const nprofile = nip19.nprofileEncode({
    pubkey: SAVJETNIK,
    relays: ["wss://relay.example"],
  });
  assert.deepEqual(extractNostrUriPubkeys(`hey nostr:${nprofile}`), [
    SAVJETNIK,
  ]);
});

test("returns each pubkey once, in first-appearance order", () => {
  const text = `nostr:${pi314Npub} and nostr:${savjetnikNpub} and nostr:${pi314Npub}`;
  assert.deepEqual(extractNostrUriPubkeys(text), [PI314, SAVJETNIK]);
});

test("stops at trailing punctuation", () => {
  assert.deepEqual(
    extractNostrUriPubkeys(`(nostr:${savjetnikNpub}), pitanje`),
    [SAVJETNIK],
  );
});

test("ignores URIs inside a code span", () => {
  assert.deepEqual(
    extractNostrUriPubkeys(`\`nostr:${savjetnikNpub} koliki je promet?\``),
    [],
  );
});

test("ignores URIs inside a fenced code block", () => {
  const text = ["```", `nostr:${savjetnikNpub}`, "```"].join("\n");
  assert.deepEqual(extractNostrUriPubkeys(text), []);
});

test("still reads a URI outside the code span in the same message", () => {
  assert.deepEqual(
    extractNostrUriPubkeys(
      `\`nostr:${pi314Npub}\` but really nostr:${savjetnikNpub}`,
    ),
    [SAVJETNIK],
  );
});

test("ignores event references, which name content and not a recipient", () => {
  const nevent = nip19.neventEncode({ id: SAVJETNIK, author: PI314 });
  const note = nip19.noteEncode(SAVJETNIK);
  assert.deepEqual(extractNostrUriPubkeys(`nostr:${nevent} nostr:${note}`), []);
});

test("ignores a truncated or corrupted entity", () => {
  assert.deepEqual(
    extractNostrUriPubkeys(`nostr:${savjetnikNpub.slice(0, 30)}`),
    [],
  );
});

test("ignores a bare npub with no nostr: prefix", () => {
  assert.deepEqual(extractNostrUriPubkeys(savjetnikNpub), []);
});

test("returns nothing for text with no URI", () => {
  assert.deepEqual(extractNostrUriPubkeys("koliki je promet danas?"), []);
});
