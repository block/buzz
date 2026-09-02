import assert from "node:assert/strict";
import test from "node:test";

import { hermesProfileNameFromAgent } from "./hermesProfileBinding.ts";

test("extracts the typed Hermes profile launcher args", () => {
  assert.equal(
    hermesProfileNameFromAgent({
      runtime: "hermes",
      agentCommand: "hermes-acp",
      agentArgs: ["--profile", "jake"],
    }),
    "jake",
  );
});

test("does not reinterpret unrelated runtime args as a Hermes binding", () => {
  assert.equal(
    hermesProfileNameFromAgent({
      runtime: "claude",
      agentCommand: "claude-agent-acp",
      agentArgs: ["--profile", "jake"],
    }),
    null,
  );
  assert.equal(
    hermesProfileNameFromAgent({
      runtime: "hermes",
      agentCommand: "hermes-acp",
      agentArgs: ["--profile", "../escape"],
    }),
    null,
  );
  assert.equal(hermesProfileNameFromAgent({ runtime: "hermes" }), null);
});
