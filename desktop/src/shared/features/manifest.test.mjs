import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const manifest = JSON.parse(
  readFileSync(
    new URL("../../../../preview-features.json", import.meta.url),
    "utf8",
  ),
);

test("thread-scoped ACP sessions is a default-off desktop experiment", () => {
  const feature = manifest.features.find(
    ({ id }) => id === "threadScopedAcpSessions",
  );

  assert.deepEqual(feature, {
    id: "threadScopedAcpSessions",
    name: "Thread Scoped ACP Sessions",
    description:
      "Give each channel thread isolated agent context. Applies when managed agents next start; DMs stay conversation-scoped.",
    platforms: ["desktop"],
  });
  assert.equal(feature.defaultEnabled, undefined);
});

test("the Workflows experiment remains unchanged", () => {
  const existing = Object.fromEntries(
    manifest.features
      .filter(({ id }) => id === "workflows")
      .map((feature) => [feature.id, feature]),
  );

  assert.deepEqual(existing, {
    workflows: {
      id: "workflows",
      name: "Workflows",
      description: "YAML-defined automations with approval gates",
      platforms: ["desktop"],
    },
  });
});

// Projects graduated out of preview, so it must not reappear as a gate: a
// stray manifest entry would hide the shipped sidebar row behind an
// experiment toggle again.
test("Projects is no longer a preview experiment", () => {
  assert.equal(
    manifest.features.find(({ id }) => id === "projects"),
    undefined,
  );
});
