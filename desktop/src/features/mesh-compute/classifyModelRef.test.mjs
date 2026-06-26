import assert from "node:assert/strict";
import test from "node:test";

import { classifyModelRef, modelRefHintLabel } from "./classifyModelRef.ts";

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

test("absolute path → local-path", () => {
  assert.deepEqual(classifyModelRef("/Users/me/models/qwen.gguf"), {
    kind: "local-path",
    path: "/Users/me/models/qwen.gguf",
  });
});

test("relative path with ./ → local-path", () => {
  assert.deepEqual(classifyModelRef("./models/qwen.gguf"), {
    kind: "local-path",
    path: "./models/qwen.gguf",
  });
});

test("home shortcut → local-path", () => {
  assert.deepEqual(classifyModelRef("~/models/qwen.gguf"), {
    kind: "local-path",
    path: "~/models/qwen.gguf",
  });
});

test(".gguf extension without path prefix → local-path", () => {
  // Bare filename ending in .gguf — user clearly means a file.
  assert.deepEqual(classifyModelRef("my-model.gguf"), {
    kind: "local-path",
    path: "my-model.gguf",
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

test("hint labels are user-facing strings", () => {
  assert.equal(
    modelRefHintLabel({ kind: "catalog", name: "x" }),
    "Looks like a catalog name",
  );
  assert.equal(
    modelRefHintLabel({ kind: "huggingface", ref: "hf://x" }),
    "HuggingFace ref",
  );
  assert.equal(
    modelRefHintLabel({ kind: "local-path", path: "/x" }),
    "Local file",
  );
  assert.equal(modelRefHintLabel({ kind: "unknown" }), null);
});
