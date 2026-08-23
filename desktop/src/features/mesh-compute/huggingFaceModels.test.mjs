import assert from "node:assert/strict";
import test from "node:test";

import {
  formatModelBytes,
  immutableHuggingFaceModelRef,
} from "./huggingFaceModels.ts";

const model = {
  repoId: "unsloth/Qwen3-GGUF",
  revision: "b17cb02dd882d5b6ab62fc777ad2995f19668350",
};

test("builds a MeshLLM-compatible immutable GGUF ref", () => {
  assert.equal(
    immutableHuggingFaceModelRef(model, {
      path: "Qwen3-Q4_K_M.gguf",
    }),
    "unsloth/Qwen3-GGUF@b17cb02dd882d5b6ab62fc777ad2995f19668350/Qwen3-Q4_K_M.gguf",
  );
});

test("rejects mutable revisions and unsafe paths", () => {
  assert.throws(() =>
    immutableHuggingFaceModelRef(
      { ...model, revision: "main" },
      { path: "model.gguf" },
    ),
  );
  assert.throws(() =>
    immutableHuggingFaceModelRef(model, { path: "../model.gguf" }),
  );
  assert.throws(() =>
    immutableHuggingFaceModelRef(model, { path: "model.safetensors" }),
  );
});

test("formats bounded model sizes", () => {
  assert.equal(formatModelBytes(5 * 1024 ** 3), "5.0 GB");
  assert.equal(formatModelBytes(512 * 1024 ** 2), "512 MB");
  assert.equal(formatModelBytes(null), null);
});
