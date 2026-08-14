import assert from "node:assert/strict";
import test from "node:test";

import { nip19 } from "nostr-tools";

import {
  isLongFormNaddr,
  NIP19_MAX_LENGTH,
  parseNaddrUri,
} from "./nostrAddress.ts";

const PUBKEY = "0".repeat(64);

function naddr(kind = 30023, identifier = "hello") {
  return `nostr:${nip19.naddrEncode({
    identifier,
    kind,
    pubkey: PUBKEY,
    relays: ["wss://relay.example"],
  })}`;
}

test("parseNaddrUri accepts nostr:naddr for kind 30023", () => {
  const parsed = parseNaddrUri(naddr());
  assert.equal(parsed?.kind, 30023);
  assert.equal(parsed?.identifier, "hello");
  assert.equal(parsed?.pubkey, PUBKEY);
  assert.deepEqual(parsed?.relays, ["wss://relay.example"]);
});

test("parseNaddrUri rejects naddr for other replaceable kinds", () => {
  assert.equal(parseNaddrUri(naddr(30024)), null);
});

test("parseNaddrUri rejects an empty long-form identifier", () => {
  assert.equal(parseNaddrUri(naddr(30023, "")), null);
});

test("parseNaddrUri rejects nsec and other NIP-19 entity types", () => {
  const nsec = `nostr:${nip19.nsecEncode(new Uint8Array(32).fill(1))}`;
  assert.equal(parseNaddrUri(nsec), null);
  assert.equal(parseNaddrUri(`nostr:${nip19.npubEncode(PUBKEY)}`), null);
});

test("parseNaddrUri enforces the NIP-19 5000 character limit", () => {
  const long = `${naddr()}${"a".repeat(NIP19_MAX_LENGTH)}`;
  assert.equal(long.length > NIP19_MAX_LENGTH, true);
  assert.equal(parseNaddrUri(long), null);
});

test("isLongFormNaddr is a non-throwing validity predicate", () => {
  assert.equal(isLongFormNaddr(naddr()), true);
  assert.equal(isLongFormNaddr("nostr:naddr1not-valid"), false);
});
