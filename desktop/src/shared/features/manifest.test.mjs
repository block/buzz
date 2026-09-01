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

test("existing Projects and Workflows experiments remain unchanged", () => {
  const existing = Object.fromEntries(
    manifest.features
      .filter(({ id }) => id === "projects" || id === "workflows")
      .map((feature) => [feature.id, feature]),
  );

  assert.deepEqual(existing, {
    projects: {
      id: "projects",
      name: "Projects",
      description: "Git repository browser and collaboration",
      platforms: ["desktop"],
    },
    workflows: {
      id: "workflows",
      name: "Workflows",
      description: "YAML-defined automations with approval gates",
      platforms: ["desktop"],
    },
  });
});
