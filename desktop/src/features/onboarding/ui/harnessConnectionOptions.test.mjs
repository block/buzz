import assert from "node:assert/strict";
import test from "node:test";

import {
  getRuntimesForConnectionMethod,
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
