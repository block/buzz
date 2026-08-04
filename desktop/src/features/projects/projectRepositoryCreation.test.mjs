import assert from "node:assert/strict";
import test from "node:test";

import {
  buildAddedRepositoryEventTemplates,
  buildAttachedRepositoryProjectEventTemplate,
  buildRepositoryChannelBindingTemplate,
  buildProjectPatchTemplate,
  buildAddedRepositoryEventTemplatesFromHead,
} from "./projectRepositoryCreation.ts";

const OWNER = "a".repeat(64);
const OTHER_OWNER = "b".repeat(64);

const project = {
  id: `${OWNER}:buzz`,
  dtag: "buzz",
  name: "Buzz",
  description: "A multi-repository workspace",
  owner: OWNER,
  createdAt: 1,
  projectChannelId: "11111111-1111-4111-8111-111111111111",
  status: "active",
  projectAddress: `30621:${OWNER}:buzz`,
  primaryRepositoryAddress: `30617:${OWNER}:desktop`,
  repositoryAddresses: [`30617:${OWNER}:desktop`, `30617:${OTHER_OWNER}:relay`],
  repositoryRelayHints: {
    [`30617:${OTHER_OWNER}:relay`]: "wss://relay.example",
  },
  repositories: [],
  unavailableRepositoryAddresses: [],
  visibility: "listed",
  legacy: false,
};

test("buildAddedRepositoryEventTemplates preserves NIP-MP project metadata and membership", () => {
  const templates = buildAddedRepositoryEventTemplates({
    accessChannelId: "11111111-1111-4111-8111-111111111111",
    project,
    ownerPubkey: OWNER,
    name: "Mobile App",
    description: "Flutter client",
    cloneUrl: "https://relay.example/git/mobile-app.git",
  });

  assert.equal(templates.repository.kind, 30617);
  assert.deepEqual(templates.repository.tags, [
    ["d", "mobile-app"],
    ["name", "Mobile App"],
    ["buzz-channel", "11111111-1111-4111-8111-111111111111"],
    ["description", "Flutter client"],
    ["clone", "https://relay.example/git/mobile-app.git"],
  ]);
  assert.equal(templates.project.kind, 30621);
  assert.deepEqual(templates.project.tags, [
    ["d", "buzz"],
    ["name", "Buzz"],
    ["description", "A multi-repository workspace"],
    ["buzz-channel", "11111111-1111-4111-8111-111111111111"],
    ["a", `30617:${OWNER}:desktop`],
    ["a", `30617:${OWNER}:mobile-app`],
    ["a", `30617:${OTHER_OWNER}:relay`, "wss://relay.example"],
  ]);
  assert.equal(templates.project.content, "");
});

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

test("buildAttachedRepositoryProjectEventTemplate links an existing repository", () => {
  const repositoryAddress = `30617:${"c".repeat(64)}:design-system`;
  const template = buildAttachedRepositoryProjectEventTemplate({
    project,
    ownerPubkey: OWNER,
    repositoryAddress,
  });

  assert.equal(template.kind, 30621);
  assert.equal(template.content, "");
  assert.deepEqual(template.tags.at(-1), ["a", repositoryAddress]);
});

test("buildAddedRepositoryEventTemplates rejects updates by another owner", () => {
  assert.throws(
    () =>
      buildAddedRepositoryEventTemplates({
        accessChannelId: "11111111-1111-4111-8111-111111111111",
        project,
        ownerPubkey: OTHER_OWNER,
        name: "Mobile",
      }),
    /Only the project owner/,
  );
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
