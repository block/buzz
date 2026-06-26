import assert from "node:assert/strict";
import test from "node:test";

import {
  UNDECRYPTABLE_DM_PLACEHOLDER,
  decryptIngestedContent,
  dmPeerPubkey,
  looksLikeNip44V2,
  makeDmIngestDecryptor,
} from "./dmCrypto.ts";

import {
  KIND_STREAM_MESSAGE,
  KIND_STREAM_MESSAGE_EDIT,
} from "@/shared/constants/kinds";

// base64(0x02 + 98 zero bytes) — 99 decoded bytes, the minimal valid NIP-44 v2
// envelope (1 version + 32 nonce + 32 MAC + 34 ciphertext floor). 132 chars.
const VALID_V2 =
  "AgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
// Same length and alphabet, but first decoded byte is 0x00, not 0x02.
const WRONG_VERSION =
  "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

// ── looksLikeNip44V2 — mirrors relay validate_nip44_v2 envelope check ─────────

test("looksLikeNip44V2 accepts a minimal valid v2 envelope", () => {
  assert.equal(looksLikeNip44V2(VALID_V2), true);
});

test("looksLikeNip44V2 rejects content with wrong version prefix", () => {
  assert.equal(looksLikeNip44V2(WRONG_VERSION), false);
});

test("looksLikeNip44V2 rejects content shorter than 99 decoded bytes", () => {
  assert.equal(looksLikeNip44V2("AgAAAAAAAAAAAAA="), false);
});

test("looksLikeNip44V2 rejects empty content", () => {
  assert.equal(looksLikeNip44V2(""), false);
});

test("looksLikeNip44V2 rejects plaintext that isn't base64-shaped", () => {
  assert.equal(looksLikeNip44V2("hey, lunch at noon?"), false);
});

test("looksLikeNip44V2 rejects base64 with a non-trailing pad char", () => {
  // '=' before the last two positions is malformed padding.
  const bad = `Ag==${VALID_V2.slice(4)}`;
  assert.equal(looksLikeNip44V2(bad), false);
});

// ── AC1: decryptIngestedContent — valid-v2-undecryptable vs malformed ─────────

const peer = "a".repeat(64);
const throwingDecrypt = async () => {
  throw new Error("nip44 decrypt failed: hmac mismatch");
};
const okDecrypt = async (_peer, ciphertext) =>
  `decrypted(${ciphertext.slice(0, 4)})`;

test("decryptIngestedContent substitutes placeholder when valid v2 ciphertext fails to decrypt", async () => {
  const body = await decryptIngestedContent(
    { kind: KIND_STREAM_MESSAGE, content: VALID_V2 },
    peer,
    throwingDecrypt,
  );
  assert.equal(body, UNDECRYPTABLE_DM_PLACEHOLDER);
});

test("decryptIngestedContent passes legacy plaintext through unchanged on decrypt failure", async () => {
  // The critical distinction: malformed / never-encrypted content must NOT
  // become the placeholder. It is not v2-shaped, so decrypt is never attempted.
  const body = await decryptIngestedContent(
    { kind: KIND_STREAM_MESSAGE, content: "hey, lunch at noon?" },
    peer,
    throwingDecrypt,
  );
  assert.equal(body, "hey, lunch at noon?");
});

test("decryptIngestedContent returns plaintext when valid v2 ciphertext decrypts", async () => {
  const body = await decryptIngestedContent(
    { kind: KIND_STREAM_MESSAGE, content: VALID_V2 },
    peer,
    okDecrypt,
  );
  assert.equal(body, "decrypted(AgAA)");
});

test("decryptIngestedContent decrypts edit-kind content (AC2 ingest side)", async () => {
  const body = await decryptIngestedContent(
    { kind: KIND_STREAM_MESSAGE_EDIT, content: VALID_V2 },
    peer,
    okDecrypt,
  );
  assert.equal(body, "decrypted(AgAA)");
});

test("decryptIngestedContent skips decrypt when there is no DM peer", async () => {
  const body = await decryptIngestedContent(
    { kind: KIND_STREAM_MESSAGE, content: VALID_V2 },
    null,
    throwingDecrypt,
  );
  assert.equal(body, VALID_V2);
});

test("decryptIngestedContent does not decrypt non-content kinds (system messages)", async () => {
  const KIND_SYSTEM_MESSAGE = 40099;
  const body = await decryptIngestedContent(
    { kind: KIND_SYSTEM_MESSAGE, content: VALID_V2 },
    peer,
    throwingDecrypt,
  );
  assert.equal(body, VALID_V2);
});

// ── AC3: dmPeerPubkey — peer = participants minus self, 2-party only ──────────

test("dmPeerPubkey returns the single non-self participant in a 2-party DM", () => {
  const channel = { channelType: "dm", participantPubkeys: ["SELF", "PEER"] };
  assert.equal(dmPeerPubkey(channel, "self"), "PEER");
});

test("dmPeerPubkey is case-insensitive on self matching", () => {
  const channel = { channelType: "dm", participantPubkeys: ["AbC", "def"] };
  assert.equal(dmPeerPubkey(channel, "abc"), "def");
});

test("dmPeerPubkey returns null for a group DM with more than one peer", () => {
  const channel = {
    channelType: "dm",
    participantPubkeys: ["self", "p1", "p2"],
  };
  assert.equal(dmPeerPubkey(channel, "self"), null);
});

test("dmPeerPubkey returns null for non-DM channels", () => {
  const channel = {
    channelType: "stream",
    participantPubkeys: ["self", "peer"],
  };
  assert.equal(dmPeerPubkey(channel, "self"), null);
});

test("dmPeerPubkey returns null when self pubkey is unknown", () => {
  const channel = { channelType: "dm", participantPubkeys: ["a", "b"] };
  assert.equal(dmPeerPubkey(channel, undefined), null);
});

// ── makeDmIngestDecryptor — no-op outside a 2-party DM ────────────────────────

test("makeDmIngestDecryptor returns events untouched outside a 2-party DM", async () => {
  const events = [
    { kind: KIND_STREAM_MESSAGE, content: VALID_V2 },
    { kind: KIND_STREAM_MESSAGE, content: "plaintext" },
  ];
  // A stream channel has no DM peer, so the decryptor must be a pure identity
  // pass-through — it never touches the IPC primitive, so this stays
  // deterministic without a Tauri mock.
  const decrypt = makeDmIngestDecryptor(
    { channelType: "stream", participantPubkeys: ["self", "peer"] },
    "self",
  );
  const result = await decrypt(events);
  assert.equal(result, events);
});

test("makeDmIngestDecryptor returns events untouched when channel is null", async () => {
  const events = [{ kind: KIND_STREAM_MESSAGE, content: VALID_V2 }];
  const decrypt = makeDmIngestDecryptor(null, "self");
  const result = await decrypt(events);
  assert.equal(result, events);
});
