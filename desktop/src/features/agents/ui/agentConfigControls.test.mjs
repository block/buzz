import assert from "node:assert/strict";
import test from "node:test";

import { resolveDefaultModelLabel } from "./agentConfigControls.tsx";

test("uses the harness-discovered default model label for an unset model", () => {
  assert.equal(
    resolveDefaultModelLabel({
      discoveredModelOptions: [
        { id: "", label: "Default model (claude-sonnet-5)" },
        { id: "claude-opus-4-8", label: "Claude Opus 4.8" },
      ],
      isSharedCompute: false,
    }),
    "Default model (claude-sonnet-5)",
  );
});

test("falls back to a generic harness default when discovery has no current model", () => {
  assert.equal(
    resolveDefaultModelLabel({
      discoveredModelOptions: [{ id: "", label: "Default model" }],
      isSharedCompute: false,
    }),
    "Default model",
  );
});

test("an explicit inherited default label wins over harness discovery", () => {
  assert.equal(
    resolveDefaultModelLabel({
      defaultModelLabel: "Default model (team-model)",
      discoveredModelOptions: [
        { id: "", label: "Default model (claude-sonnet-5)" },
      ],
      isSharedCompute: false,
    }),
    "Default model (team-model)",
  );
});
