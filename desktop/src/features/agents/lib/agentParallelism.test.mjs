import assert from "node:assert/strict";
import test from "node:test";

import {
  DEFAULT_AGENT_PARALLELISM,
  resolveAgentParallelism,
} from "./agentParallelism.ts";

test("parallelism uses the app default only when input and definition omit it", () => {
  assert.equal(
    resolveAgentParallelism(undefined, undefined),
    DEFAULT_AGENT_PARALLELISM,
  );
  assert.equal(
    resolveAgentParallelism(undefined, null),
    DEFAULT_AGENT_PARALLELISM,
  );
  assert.equal(resolveAgentParallelism(undefined, 4), 4);
  assert.equal(resolveAgentParallelism(2, 4), 2);
});
