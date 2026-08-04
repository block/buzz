import assert from "node:assert/strict";
import test from "node:test";

import {
  buildAddedRepositoryEventTemplates,
  buildAttachedRepositoryProjectEventTemplate,
  buildRepositoryChannelBindingTemplate,
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
