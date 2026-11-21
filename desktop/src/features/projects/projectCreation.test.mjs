import assert from "node:assert/strict";
import test from "node:test";

import {
  buildDefaultProjectRepositoryTemplate,
  buildProjectAnnouncementTemplate,
  buildProjectBootstrapTemplates,
  conflictingListedProject,
  isUnsupportedProjectKindError,
} from "./projectCreation.ts";

const OWNER = "a".repeat(64);
const CHANNEL = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";

test("buildProjectAnnouncementTemplate emits a channel-first NIP-MP project", () => {
  const templates = buildProjectAnnouncementTemplate({
    description: "A workspace that starts as a conversation",
    name: "Buzz",
    ownerPubkey: OWNER,
    projectChannelId: CHANNEL,
  });

  assert.equal(templates.dtag, "buzz");
  assert.equal(templates.project.kind, 30621);
  assert.deepEqual(templates.project.tags, [
    ["d", "buzz"],
    ["name", "Buzz"],
    ["buzz-channel", CHANNEL],
    ["description", "A workspace that starts as a conversation"],
  ]);
  assert.equal(templates.project.content, "");
  assert.equal(
    templates.project.tags.some((tag) => tag[0] === "a"),
    false,
  );
});

test("buildProjectAnnouncementTemplate records unlisted visibility and members", () => {
  const address = `30617:${OWNER}:buzz`;
  const templates = buildProjectAnnouncementTemplate({
    name: "Buzz",
    ownerPubkey: OWNER,
    projectChannelId: CHANNEL,
    projectVisibility: "unlisted",
    repositoryAddresses: [address],
  });

  assert.deepEqual(templates.project.tags, [
    ["d", "buzz"],
    ["name", "Buzz"],
    ["buzz-channel", CHANNEL],
    ["buzz-visibility", "unlisted"],
    ["a", address],
  ]);
});

test("buildProjectBootstrapTemplates binds a default repository to the home channel", () => {
  const templates = buildProjectBootstrapTemplates({
    description: "A workspace that starts as a conversation",
    name: "Buzz",
    ownerPubkey: OWNER,
    projectChannelId: CHANNEL,
  });
  const repositoryAddress = `30617:${OWNER}:buzz`;

  assert.equal(templates.dtag, "buzz");
  assert.equal(templates.repositoryAddress, repositoryAddress);
  assert.equal(templates.project.kind, 30621);
  assert.equal(templates.repository.kind, 30617);
  assert.deepEqual(templates.project.tags, [
    ["d", "buzz"],
    ["name", "Buzz"],
    ["buzz-channel", CHANNEL],
    ["description", "A workspace that starts as a conversation"],
    ["a", repositoryAddress],
  ]);
  assert.deepEqual(templates.repository.tags, [
    ["d", "buzz"],
    ["name", "Buzz"],
    ["buzz-channel", CHANNEL],
    ["description", "A workspace that starts as a conversation"],
  ]);
});

test("buildDefaultProjectRepositoryTemplate uses the project slug as the repo id", () => {
  const template = buildDefaultProjectRepositoryTemplate({
    name: "Space Invaders 3D",
    ownerPubkey: OWNER,
    projectChannelId: CHANNEL,
  });

  assert.equal(template.dtag, "space-invaders-3d");
  assert.equal(template.repositoryAddress, `30617:${OWNER}:space-invaders-3d`);
});

test("buildProjectAnnouncementTemplate rejects names without an identifier", () => {
  assert.throws(
    () =>
      buildProjectAnnouncementTemplate({
        name: "!!!",
        ownerPubkey: OWNER,
        projectChannelId: CHANNEL,
      }),
    /letters or numbers/,
  );
});

test("buildProjectAnnouncementTemplate enforces the description tag byte limit", () => {
  assert.doesNotThrow(() =>
    buildProjectAnnouncementTemplate({
      description: "🙂".repeat(512),
      name: "Buzz",
      ownerPubkey: OWNER,
      projectChannelId: CHANNEL,
    }),
  );
  assert.throws(
    () =>
      buildProjectAnnouncementTemplate({
        description: "🙂".repeat(513),
        name: "Buzz",
        ownerPubkey: OWNER,
        projectChannelId: CHANNEL,
      }),
    /2,048 bytes/,
  );
});

test("buildProjectAnnouncementTemplate rejects an invalid project channel", () => {
  assert.throws(
    () =>
      buildProjectAnnouncementTemplate({
        name: "Buzz",
        ownerPubkey: OWNER,
        projectChannelId: "not-a-channel",
      }),
    /Project channel is invalid/,
  );
});

test("conflictingListedProject ignores the caller's own slug and legacy cards", () => {
  assert.equal(
    conflictingListedProject(
      [
        {
          dtag: "buzz",
          legacy: false,
          name: "Buzz",
          owner: OWNER,
        },
      ],
      { dtag: "buzz", name: "Buzz", ownerPubkey: OWNER },
    ),
    null,
  );
  assert.equal(
    conflictingListedProject(
      [
        {
          dtag: "buzz",
          legacy: true,
          name: "Buzz",
          owner: "b".repeat(64),
        },
      ],
      { dtag: "buzz", name: "Buzz", ownerPubkey: OWNER },
    ),
    null,
  );
});

test("conflictingListedProject blocks another listed project with the same name or slug", () => {
  const other = {
    dtag: "space-invaders-3d",
    legacy: false,
    name: "Space Invaders 3D",
    owner: "b".repeat(64),
  };
  assert.deepEqual(
    conflictingListedProject([other], {
      dtag: "space-invaders-3d",
      name: "Space Invaders 3D",
      ownerPubkey: OWNER,
    }),
    other,
  );
  assert.deepEqual(
    conflictingListedProject([other], {
      dtag: "space-invaders-3d-remake",
      name: "Space Invaders 3D",
      ownerPubkey: OWNER,
    }),
    other,
  );
});

test("isUnsupportedProjectKindError recognizes relay kind compatibility failures", () => {
  assert.equal(
    isUnsupportedProjectKindError(
      new Error("restricted: unknown event kind 30621"),
    ),
    true,
  );
  assert.equal(
    isUnsupportedProjectKindError(new Error("mock project event rejection")),
    false,
  );
});
