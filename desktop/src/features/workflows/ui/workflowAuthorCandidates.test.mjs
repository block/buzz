import assert from "node:assert/strict";
import { test } from "node:test";
import { npubEncode } from "nostr-tools/nip19";

import {
  enrichAuthorCandidates,
  mergeAuthorCandidateSources,
  normalizeAuthorPubkey,
  parseDirectAuthorInput,
} from "./workflowAuthorCandidates.ts";

const A = "a".repeat(64);
const B = "b".repeat(64);
const C = "c".repeat(64);

test("normalizes only valid 64-character hex candidate identities", () => {
  assert.equal(normalizeAuthorPubkey(`  ${A.toUpperCase()}  `), A);
  assert.equal(normalizeAuthorPubkey(A.slice(1)), null);
  assert.equal(normalizeAuthorPubkey("z".repeat(64)), null);
  assert.equal(normalizeAuthorPubkey(npubEncode(A)), null);
});

test("parses direct hex and npub input to normalized hex", () => {
  assert.equal(parseDirectAuthorInput(A.toUpperCase()), A);
  assert.equal(parseDirectAuthorInput(`  ${npubEncode(A)}  `), A);
  assert.equal(parseDirectAuthorInput("not-a-pubkey"), null);
});

test("merges valid candidates with stable first-wins source priority", () => {
  const merged = mergeAuthorCandidateSources([
    [
      { pubkey: B.toUpperCase(), displayName: "Channel B" },
      { pubkey: "invalid", displayName: "Invalid" },
      { pubkey: A, displayName: "Channel A", isAgent: true },
    ],
    [
      { pubkey: A.toUpperCase(), displayName: "Directory A" },
      { pubkey: C, displayName: "Directory C" },
      { pubkey: B, displayName: "Directory B" },
    ],
  ]);

  assert.deepEqual(
    merged.map(({ pubkey, displayName }) => ({ pubkey, displayName })),
    [
      { pubkey: B, displayName: "Channel B" },
      { pubkey: A, displayName: "Channel A" },
      { pubkey: C, displayName: "Directory C" },
    ],
  );
  assert.equal(merged[1].isAgent, true);
});

test("profile enrichment updates matching presentation without reordering", () => {
  const candidates = mergeAuthorCandidateSources([
    [
      {
        pubkey: B,
        displayName: "Roster B",
        nip05Handle: "b@old.example",
      },
      { pubkey: A, displayName: "Roster A" },
      { pubkey: C, displayName: "Roster C" },
    ],
  ]);

  const enriched = enrichAuthorCandidates(candidates, {
    [A.toUpperCase()]: {
      displayName: "Profile A",
      avatarUrl: "https://example.test/a.png",
      nip05Handle: null,
      ownerPubkey: B.toUpperCase(),
      isAgent: true,
    },
    [B]: {
      displayName: null,
      name: "Profile B name",
      avatarUrl: null,
      nip05Handle: null,
      ownerPubkey: null,
    },
    ["d".repeat(64)]: {
      displayName: "Not a candidate",
      avatarUrl: null,
      nip05Handle: null,
      ownerPubkey: null,
    },
  });

  assert.deepEqual(
    enriched.map(({ pubkey }) => pubkey),
    [B, A, C],
  );
  assert.equal(enriched[0].displayName, "Profile B name");
  assert.equal(enriched[0].nip05Handle, "b@old.example");
  assert.equal(enriched[1].displayName, "Profile A");
  assert.equal(enriched[1].avatarUrl, "https://example.test/a.png");
  assert.equal(enriched[1].ownerPubkey, B);
  assert.equal(enriched[1].isAgent, true);
  assert.strictEqual(enriched[2], candidates[2]);
});
