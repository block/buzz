import assert from "node:assert/strict";
import test from "node:test";

import { classifyModelRef } from "./classifyModelRef.ts";

test("empty string → unknown", () => {
  assert.deepEqual(classifyModelRef(""), { kind: "unknown" });
  assert.deepEqual(classifyModelRef("   "), { kind: "unknown" });
});

test("hf:// prefix → huggingface", () => {
  assert.deepEqual(classifyModelRef("hf://meshllm/qwen3-8b@main"), {
    kind: "huggingface",
    ref: "hf://meshllm/qwen3-8b@main",
  });
});

// The vendored mesh-llm SDK's `parse_exact_model_ref` has no local-path
// variant — feeding any of these into `mesh_start_node` always fails with
// "Expected an exact model ref" at runtime. We therefore classify them as
// unknown so the UI keeps the Start button disabled with a clear signal
// instead of letting the submit bubble up a cryptic SDK error.
// See https://github.com/block/buzz/issues/4049.

test("absolute path → unknown", () => {
  assert.deepEqual(classifyModelRef("/Users/me/models/qwen.gguf"), {
    kind: "unknown",
  });
});

test("relative path with ./ → unknown", () => {
  assert.deepEqual(classifyModelRef("./models/qwen.gguf"), { kind: "unknown" });
});

test("relative path with ../ → unknown", () => {
  assert.deepEqual(classifyModelRef("../models/qwen.gguf"), { kind: "unknown" });
});

test("home shortcut → unknown", () => {
  assert.deepEqual(classifyModelRef("~/models/qwen.gguf"), { kind: "unknown" });
});

test(".gguf extension without path prefix → unknown", () => {
  assert.deepEqual(classifyModelRef("my-model.gguf"), { kind: "unknown" });
});

test("file:// URL → unknown", () => {
  assert.deepEqual(classifyModelRef("file:/// Users/me/models/qwen.gguf"), {
    kind: "unknown",
  });
});

test("plain name → catalog", () => {
  assert.deepEqual(classifyModelRef("Qwen3-8B-Q4_K_M"), {
    kind: "catalog",
    name: "Qwen3-8B-Q4_K_M",
  });
});

test("trims whitespace before classifying", () => {
  assert.deepEqual(classifyModelRef("  Qwen3-8B-Q4_K_M  "), {
    kind: "catalog",
    name: "Qwen3-8B-Q4_K_M",
  });
});
