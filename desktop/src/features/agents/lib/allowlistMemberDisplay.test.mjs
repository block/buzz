import assert from "node:assert/strict";
import test from "node:test";

import {
  resolveAllowlistChipAvatar,
  resolveAllowlistChipIsAgent,
  resolveAllowlistChipLabel,
} from "./allowlistMemberDisplay.ts";

const PUBKEY =
  "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";

test("resolveAllowlistChipLabel prefers search hint display name", () => {
  assert.equal(
    resolveAllowlistChipLabel({
      pubkey: PUBKEY,
      hint: {
        displayName: "Aviz",
        avatarUrl: null,
        nip05Handle: "aviz@example.com",
        isAgent: false,
      },
    }),
    "Aviz",
  );
});

test("resolveAllowlistChipLabel falls back to nip05 from hint", () => {
  assert.equal(
    resolveAllowlistChipLabel({
      pubkey: PUBKEY,
      hint: {
        displayName: null,
        avatarUrl: null,
        nip05Handle: "aviz@example.com",
        isAgent: false,
      },
    }),
    "aviz@example.com",
  );
});

test("resolveAllowlistChipLabel resolves batch profiles when hint is missing", () => {
  assert.equal(
    resolveAllowlistChipLabel({
      pubkey: PUBKEY,
      profiles: {
        [PUBKEY]: {
          displayName: "Aviz-Agent",
          avatarUrl: null,
          nip05Handle: null,
          ownerPubkey: null,
          isAgent: true,
        },
      },
    }),
    "Aviz-Agent",
  );
});

test("resolveAllowlistChipLabel truncates pubkey when nothing else is known", () => {
  assert.equal(
    resolveAllowlistChipLabel({ pubkey: PUBKEY }),
    "abcdef01…6789",
  );
});

test("resolveAllowlistChipLabel prefers hint over stale batch profile", () => {
  assert.equal(
    resolveAllowlistChipLabel({
      pubkey: PUBKEY,
      hint: {
        displayName: "Fresh Name",
        avatarUrl: null,
        nip05Handle: null,
        isAgent: false,
      },
      profiles: {
        [PUBKEY]: {
          displayName: "Stale Name",
          avatarUrl: null,
          nip05Handle: null,
          ownerPubkey: null,
        },
      },
    }),
    "Fresh Name",
  );
});

test("resolveAllowlistChipAvatar and isAgent merge hint with batch profiles", () => {
  assert.equal(
    resolveAllowlistChipAvatar({
      pubkey: PUBKEY,
      profiles: {
        [PUBKEY]: {
          displayName: "Aviz",
          avatarUrl: "https://example.com/a.png",
          nip05Handle: null,
          ownerPubkey: null,
        },
      },
    }),
    "https://example.com/a.png",
  );
  assert.equal(
    resolveAllowlistChipIsAgent({
      pubkey: PUBKEY,
      hint: {
        displayName: "Aviz-Agent",
        avatarUrl: null,
        nip05Handle: null,
        isAgent: true,
      },
    }),
    true,
  );
});
