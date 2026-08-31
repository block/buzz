import assert from "node:assert/strict";
import test from "node:test";

import { effectiveAgentDescription } from "./agentDescription.ts";

test("an authored description wins", () => {
  assert.equal(
    effectiveAgentDescription({ description: "Reviews desktop PRs." }),
    "Reviews desktop PRs.",
  );
});

test("an authored description is trimmed", () => {
  assert.equal(
    effectiveAgentDescription({ description: "  Reviews desktop PRs.  " }),
    "Reviews desktop PRs.",
  );
});

test("blank, whitespace-only, and missing descriptions yield null", () => {
  assert.equal(effectiveAgentDescription({ description: "" }), null);
  assert.equal(effectiveAgentDescription({ description: "   " }), null);
  assert.equal(effectiveAgentDescription({ description: null }), null);
  assert.equal(effectiveAgentDescription({}), null);
});
