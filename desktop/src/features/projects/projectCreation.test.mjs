import assert from "node:assert/strict";
import test from "node:test";

import {
  buildInitialProjectEventTemplates,
  isUnsupportedProjectKindError,
} from "./projectCreation.ts";

const OWNER = "a".repeat(64);
const CHANNEL = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";

test("buildInitialProjectEventTemplates emits a NIP-MP project", () => {
  const templates = buildInitialProjectEventTemplates({
    accessChannelId: CHANNEL,
    cloneUrl: "https://relay.example/git/owner/sprout.git",
    description: "A multi-repository workspace",
    name: "Sprout",
    ownerPubkey: OWNER,
    webUrl: "https://example.com/sprout",
  });

  assert.equal(templates.dtag, "sprout");
  assert.equal(templates.project.kind, 30621);
  assert.equal(templates.repository.kind, 30617);
  assert.deepEqual(templates.project.tags, [
    ["d", "sprout"],
    ["name", "Sprout"],
    ["buzz-channel", CHANNEL],
    ["description", "A multi-repository workspace"],
    ["a", `30617:${OWNER}:sprout`],
  ]);
  assert.equal(templates.project.content, "");
  assert.deepEqual(templates.repository.tags, [
    ["d", "sprout"],
    ["name", "Sprout"],
    ["buzz-channel", CHANNEL],
    ["description", "A multi-repository workspace"],
    ["clone", "https://relay.example/git/owner/sprout.git"],
    ["web", "https://example.com/sprout"],
  ]);
});

test("buildInitialProjectEventTemplates rejects names without an identifier", () => {
  assert.throws(
    () =>
      buildInitialProjectEventTemplates({
        accessChannelId: CHANNEL,
        name: "!!!",
        ownerPubkey: OWNER,
      }),
    /letters or numbers/,
  );
});

test("buildInitialProjectEventTemplates enforces the description tag byte limit", () => {
  assert.doesNotThrow(() =>
    buildInitialProjectEventTemplates({
      accessChannelId: CHANNEL,
      description: "🙂".repeat(512),
      name: "Sprout",
      ownerPubkey: OWNER,
    }),
  );
  assert.throws(
    () =>
      buildInitialProjectEventTemplates({
        accessChannelId: CHANNEL,
        description: "🙂".repeat(513),
        name: "Sprout",
        ownerPubkey: OWNER,
      }),
    /2,048 bytes/,
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
