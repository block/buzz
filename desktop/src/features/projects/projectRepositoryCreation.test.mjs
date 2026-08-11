import assert from "node:assert/strict";
import test from "node:test";

import {
  buildRepositoryChannelBindingTemplate,
  buildProjectPatchTemplate,
  buildAddedRepositoryEventTemplatesFromHead,
} from "./projectRepositoryCreation.ts";
import { validateProjectEventEnvelope } from "./projectModels.ts";

const OWNER = "a".repeat(64);

test("buildRepositoryChannelBindingTemplate preserves repository metadata", () => {
  const repository = {
    id: `${OWNER}:desktop`,
    dtag: "desktop",
    name: "Desktop",
    description: "Desktop app",
    owner: OWNER,
    createdAt: 1,
    repoAddress: `30617:${OWNER}:desktop`,
    eventContent: "Desktop app",
    eventTags: [
      ["d", "desktop"],
      ["name", "Desktop"],
      ["x-custom", "preserve-me"],
    ],
  };
  const template = buildRepositoryChannelBindingTemplate({
    channelId: "11111111-1111-4111-8111-111111111111",
    ownerPubkey: OWNER,
    repository,
  });

  assert.equal(template.content, "Desktop app");
  assert.deepEqual(template.tags, [
    ["d", "desktop"],
    ["name", "Desktop"],
    ["x-custom", "preserve-me"],
    ["buzz-channel", "11111111-1111-4111-8111-111111111111"],
  ]);
});

// ── buildProjectPatchTemplate: unknown-tag preservation ────────────────────

test("buildProjectPatchTemplate preserves unknown tags from the live head", () => {
  const OWNER = "a".repeat(64);
  const existingAddress = `30617:${OWNER}:desktop`;
  const liveHead = {
    id: "e".repeat(64),
    kind: 30621,
    pubkey: OWNER,
    created_at: 100,
    content: "ignored-by-nip-mp",
    tags: [
      ["d", "platform"],
      ["name", "Platform"],
      ["description", "Multi-repo project"],
      ["buzz-channel", "11111111-1111-4111-8111-111111111111"],
      ["future-metadata", "preserve-me"],
      ["alt", "extension tag that must survive round-trip"],
      ["a", existingAddress],
    ],
  };
  const newAddress = `30617:${OWNER}:mobile`;
  const template = buildProjectPatchTemplate({
    liveHead,
    ownerPubkey: OWNER,
    repositoryAddresses: [existingAddress, newAddress],
  });

  // Content must be preserved verbatim (NIP-MP §content).
  assert.equal(template.content, "ignored-by-nip-mp");

  // Non-`a` tags — including unknown ones — must survive in order.
  const nonMemberTags = template.tags.filter((t) => t[0] !== "a");
  assert.deepEqual(nonMemberTags, [
    ["d", "platform"],
    ["name", "Platform"],
    ["description", "Multi-repo project"],
    ["buzz-channel", "11111111-1111-4111-8111-111111111111"],
    ["future-metadata", "preserve-me"],
    ["alt", "extension tag that must survive round-trip"],
  ]);

  // New address must be added; sorted order maintained.
  const memberTags = template.tags.filter((t) => t[0] === "a").map((t) => t[1]);
  assert.deepEqual(memberTags.sort(), [existingAddress, newAddress].sort());
});

test("buildProjectPatchTemplate preserves relay hints on existing members", () => {
  const OWNER = "a".repeat(64);
  const OTHER = "b".repeat(64);
  const existingWithHint = `30617:${OTHER}:relay`;
  const liveHead = {
    id: "e".repeat(64),
    kind: 30621,
    pubkey: OWNER,
    created_at: 100,
    content: "",
    tags: [
      ["d", "platform"],
      ["a", existingWithHint, "wss://relay.example"],
    ],
  };
  const newAddress = `30617:${OWNER}:mobile`;
  const template = buildProjectPatchTemplate({
    liveHead,
    ownerPubkey: OWNER,
    repositoryAddresses: [existingWithHint, newAddress],
  });

  const existingTag = template.tags.find(
    (t) => t[0] === "a" && t[1] === existingWithHint,
  );
  assert.ok(existingTag, "existing member tag must be present");
  assert.equal(
    existingTag[2],
    "wss://relay.example",
    "relay hint must be preserved",
  );
});

test("buildProjectPatchTemplate rejects a non-owner caller", () => {
  const OWNER = "a".repeat(64);
  const OTHER = "b".repeat(64);
  const liveHead = {
    id: "e".repeat(64),
    kind: 30621,
    pubkey: OWNER,
    created_at: 100,
    content: "",
    tags: [["d", "platform"]],
  };
  assert.throws(
    () =>
      buildProjectPatchTemplate({
        liveHead,
        ownerPubkey: OTHER,
        repositoryAddresses: [],
      }),
    /Only the project owner/,
  );
});

test("buildProjectPatchTemplate rejects a repository address list exceeding 64 members", () => {
  const OWNER = "a".repeat(64);
  const liveHead = {
    id: "e".repeat(64),
    kind: 30621,
    pubkey: OWNER,
    created_at: 100,
    content: "",
    tags: [["d", "wide"]],
  };
  const tooMany = Array.from(
    { length: 65 },
    (_, i) => `30617:${"a".repeat(64)}:repo-${String(i).padStart(2, "0")}`,
  );
  assert.throws(
    () =>
      buildProjectPatchTemplate({
        liveHead,
        ownerPubkey: OWNER,
        repositoryAddresses: tooMany,
      }),
    /64/,
  );
});

// ── buildAddedRepositoryEventTemplatesFromHead: racing writer ───────────────

test("buildAddedRepositoryEventTemplatesFromHead detects a concurrent add via the live head", () => {
  const OWNER = "a".repeat(64);
  const existingAddress = `30617:${OWNER}:desktop`;
  const newDtag = "mobile";
  const newAddress = `30617:${OWNER}:${newDtag}`;

  // Live head already contains the address (another session snuck it in).
  const liveHeadWithRace = {
    id: "e".repeat(64),
    kind: 30621,
    pubkey: OWNER,
    created_at: 150,
    content: "",
    tags: [
      ["d", "platform"],
      ["a", existingAddress],
      ["a", newAddress], // concurrent add
    ],
  };
  assert.throws(
    () =>
      buildAddedRepositoryEventTemplatesFromHead({
        accessChannelId: "11111111-1111-4111-8111-111111111111",
        existingRepositoryAddresses: [existingAddress],
        liveHead: liveHeadWithRace,
        name: "Mobile",
        ownerPubkey: OWNER,
      }),
    /already contains.*mobile.*another session/,
  );
});

// ── validateProjectEventEnvelope: shared full-envelope validator ─────────────
// These tests pin the NIP-MP validation rules through the WRITE helper so that
// a nonconforming live head (e.g. from a permissive relay) is caught before
// Desktop signs and re-submits it.

test("validateProjectEventEnvelope accepts a valid minimal envelope", () => {
  assert.doesNotThrow(() =>
    validateProjectEventEnvelope([["d", "platform"]], ""),
  );
});

test("validateProjectEventEnvelope rejects missing d tag", () => {
  assert.throws(
    () => validateProjectEventEnvelope([["name", "X"]], ""),
    /NIP-MP.*'d'/,
  );
});

test("validateProjectEventEnvelope rejects duplicate d tags", () => {
  assert.throws(
    () =>
      validateProjectEventEnvelope(
        [
          ["d", "a"],
          ["d", "b"],
        ],
        "",
      ),
    /NIP-MP.*'d'/,
  );
});

test("validateProjectEventEnvelope rejects duplicate name tags", () => {
  assert.throws(
    () =>
      validateProjectEventEnvelope(
        [
          ["d", "platform"],
          ["name", "X"],
          ["name", "Y"],
        ],
        "",
      ),
    /NIP-MP.*duplicate.*'name'/,
  );
});

test("validateProjectEventEnvelope rejects a name tag that exceeds 256 bytes", () => {
  const longName = "x".repeat(257);
  assert.throws(
    () =>
      validateProjectEventEnvelope(
        [
          ["d", "platform"],
          ["name", longName],
        ],
        "",
      ),
    /NIP-MP.*'name'.*256/,
  );
});

test("validateProjectEventEnvelope rejects a description tag that exceeds 2048 bytes", () => {
  const longDesc = "x".repeat(2049);
  assert.throws(
    () =>
      validateProjectEventEnvelope(
        [
          ["d", "platform"],
          ["description", longDesc],
        ],
        "",
      ),
    /NIP-MP.*'description'.*2048/,
  );
});

test("validateProjectEventEnvelope rejects more than 64 a-tags", () => {
  const tags = [["d", "wide"]];
  for (let i = 0; i < 65; i++) {
    tags.push([
      "a",
      `30617:${"a".repeat(64)}:repo-${String(i).padStart(2, "0")}`,
    ]);
  }
  assert.throws(() => validateProjectEventEnvelope(tags, ""), /NIP-MP.*64/);
});

test("validateProjectEventEnvelope rejects a member address with uppercase owner hex", () => {
  const upperAddr = `30617:${"A".repeat(64)}:desktop`;
  assert.throws(
    () =>
      validateProjectEventEnvelope(
        [
          ["d", "platform"],
          ["a", upperAddr],
        ],
        "",
      ),
    /NIP-MP.*invalid.*address/,
  );
});

test("validateProjectEventEnvelope rejects duplicate a-tag addresses", () => {
  const addr = `30617:${"a".repeat(64)}:desktop`;
  assert.throws(
    () =>
      validateProjectEventEnvelope(
        [
          ["d", "platform"],
          ["a", addr],
          ["a", addr],
        ],
        "",
      ),
    /NIP-MP.*duplicate.*address/,
  );
});

test("buildProjectPatchTemplate catches duplicate d in live head via full-envelope validation", () => {
  // A relay that accepted a nonconforming event could serve a head with two d tags.
  // buildProjectPatchTemplate must catch this before signing.
  const badLiveHead = {
    id: "e".repeat(64),
    kind: 30621,
    pubkey: OWNER,
    created_at: 100,
    content: "",
    tags: [
      ["d", "platform"],
      ["d", "extra"],
    ],
  };
  assert.throws(
    () =>
      buildProjectPatchTemplate({
        liveHead: badLiveHead,
        ownerPubkey: OWNER,
        repositoryAddresses: [],
      }),
    /NIP-MP.*'d'/,
  );
});
