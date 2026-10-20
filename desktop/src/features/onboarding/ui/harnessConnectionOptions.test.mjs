import assert from "node:assert/strict";
import test from "node:test";

import {
  getRuntimesForConnectionMethod,
  orderRuntimesForConnectionMethod,
  runtimeSupportsConnectionMethod,
} from "./harnessConnectionOptions.ts";

const runtimes = [
  { id: "claude" },
  { id: "codex" },
  { id: "buzz-agent" },
  { id: "goose" },
  { id: "cursor" },
  { id: "openclaw" },
  { id: "custom" },
];

test("subscription and API choices expose the prototype catalog groups", () => {
  assert.deepEqual(
    getRuntimesForConnectionMethod(runtimes, "subscription").map(
      ({ id }) => id,
    ),
    ["claude", "codex", "cursor"],
  );
  assert.deepEqual(
    getRuntimesForConnectionMethod(runtimes, "api").map(({ id }) => id),
    ["buzz-agent", "goose", "openclaw"],
  );
});

test("custom harnesses are not assigned an onboarding connection method", () => {
  assert.equal(
    runtimeSupportsConnectionMethod("custom", "subscription"),
    false,
  );
  assert.equal(runtimeSupportsConnectionMethod("custom", "api"), false);
});

test("runtime ordering keeps one contiguous not-installed section", () => {
  const mixed = [
    { id: "buzz-agent", availability: "not_installed" },
    { id: "goose", availability: "available" },
    { id: "openclaw", availability: "not_installed" },
    { id: "opencode", availability: "available" },
  ];

  assert.deepEqual(
    orderRuntimesForConnectionMethod(mixed, "api").map(
      ({ id, availability }) => `${id}:${availability}`,
    ),
    [
      "goose:available",
      "opencode:available",
      "buzz-agent:not_installed",
      "openclaw:not_installed",
    ],
  );
});
