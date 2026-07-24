import assert from "node:assert/strict";
import test from "node:test";

import {
  MODEL_NO_MODELS_VALUE,
  appendNoModelsSentinel,
  resolveDefaultModelLabel,
  resolveModelFieldStatusMessage,
} from "./agentConfigControls.tsx";

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

// ── appendNoModelsSentinel ─────────────────────────────────────────────────────

test("appendNoModelsSentinel_emptyOptionsDiscoveryFinished_addsDisabledRow", () => {
  const options = appendNoModelsSentinel([], false);
  assert.equal(options.length, 1);
  assert.equal(options[0].disabled, true);
  assert.equal(options[0].label, "No models found");
  assert.equal(options[0].value, MODEL_NO_MODELS_VALUE);
});

test("appendNoModelsSentinel_emptyOptionsDiscoveryLoading_doesNotAddRow", () => {
  const options = appendNoModelsSentinel([], true);
  assert.equal(options.length, 0);
});

test("appendNoModelsSentinel_nonEmptyOptionsDiscoveryFinished_doesNotAddRow", () => {
  const options = appendNoModelsSentinel(
    [{ label: "Default model", value: "" }],
    false,
  );
  assert.equal(options.length, 1);
  assert.equal(options[0].label, "Default model");
});

test("model status omits provider selection guidance before discovery", () => {
  assert.equal(
    resolveModelFieldStatusMessage({
      discoveredModelOptions: null,
      loading: false,
      status: null,
    }),
    null,
  );
});

test("model status preserves loading, discovery, and saved-state messages", () => {
  assert.equal(
    resolveModelFieldStatusMessage({
      discoveredModelOptions: null,
      loading: true,
      status: null,
    }),
    "Loading models...",
  );
  assert.equal(
    resolveModelFieldStatusMessage({
      discoveredModelOptions: null,
      loading: false,
      status: { message: "Couldn't load models", tone: "warning" },
    }),
    "Couldn't load models",
  );
  assert.equal(
    resolveModelFieldStatusMessage({
      discoveredModelOptions: [],
      loading: false,
      status: null,
    }),
    "Saved changes take effect on the next start.",
  );
});
