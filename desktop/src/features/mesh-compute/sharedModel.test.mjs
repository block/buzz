import assert from "node:assert/strict";
import test from "node:test";

import { isSharedModelRef, sharedModelShortName } from "./sharedModel.ts";

/** Minimal catalog fixture with one solo and two shared models. */
const catalog = {
  gpuName: "Test GPU",
  vramDisplay: "24 GB",
  vramGb: 24,
  recommended: "Qwen3-8B-Q4_K_M",
  entries: [
    {
      name: "Qwen3-8B-Q4_K_M",
      size: "5.0GB",
      sizeGb: 5,
      description: "",
      fit: "comfortable",
      installed: false,
      recommended: true,
      shared: false,
      estimatedMembers: null,
    },
  ],
  shared: [
    {
      name: "meshllm/Qwen3-8B-Q4_K_M-layers",
      size: "5.0GB",
      sizeGb: 5,
      description: "",
      fit: "too_large",
      installed: false,
      recommended: false,
      shared: true,
      estimatedMembers: 2,
    },
    {
      name: "meshllm/Qwen3-235B-A22B-UD-Q4_K_XL-layers",
      size: "134GB",
      sizeGb: 134,
      description: "",
      fit: "too_large",
      installed: false,
      recommended: false,
      shared: true,
      estimatedMembers: 7,
    },
  ],
};

test("isSharedModelRef: matches a shared model by exact name", () => {
  assert.equal(
    isSharedModelRef("meshllm/Qwen3-235B-A22B-UD-Q4_K_XL-layers", catalog),
    true,
  );
});

test("isSharedModelRef: trims whitespace before matching", () => {
  assert.equal(
    isSharedModelRef("  meshllm/Qwen3-8B-Q4_K_M-layers  ", catalog),
    true,
  );
});

test("isSharedModelRef: a solo model is not shared", () => {
  assert.equal(isSharedModelRef("Qwen3-8B-Q4_K_M", catalog), false);
});

test("isSharedModelRef: unknown ref is not shared", () => {
  assert.equal(isSharedModelRef("some/other-model", catalog), false);
});

test("isSharedModelRef: empty ref is not shared", () => {
  assert.equal(isSharedModelRef("", catalog), false);
  assert.equal(isSharedModelRef("   ", catalog), false);
});

test("isSharedModelRef: null catalog is not shared", () => {
  assert.equal(isSharedModelRef("meshllm/Qwen3-8B-Q4_K_M-layers", null), false);
});

test("sharedModelShortName: strips meshllm/ prefix and -layers suffix", () => {
  assert.equal(
    sharedModelShortName("meshllm/Qwen3-235B-A22B-UD-Q4_K_XL-layers"),
    "Qwen3-235B-A22B-UD-Q4_K_XL",
  );
});

test("sharedModelShortName: leaves a plain name unchanged", () => {
  assert.equal(sharedModelShortName("Qwen3-8B-Q4_K_M"), "Qwen3-8B-Q4_K_M");
});
