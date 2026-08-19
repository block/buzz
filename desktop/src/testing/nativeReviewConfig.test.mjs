import assert from "node:assert/strict";
import { test } from "node:test";

import { parseNativeReviewRelay } from "./nativeReviewConfig.ts";

for (const host of ["localhost", "127.0.0.1", "[::1]"]) {
  test(`native review relay accepts bounded explicit port for ${host}`, () => {
    assert.equal(parseNativeReviewRelay(`ws://${host}:3030`)?.port, "3030");
  });

  test(`native review relay rejects port zero for ${host}`, () => {
    assert.equal(parseNativeReviewRelay(`ws://${host}:0`), null);
  });

  test(`native review relay rejects a missing port for ${host}`, () => {
    assert.equal(parseNativeReviewRelay(`ws://${host}`), null);
  });

  test(`native review relay rejects an out-of-range port for ${host}`, () => {
    assert.equal(parseNativeReviewRelay(`ws://${host}:65536`), null);
  });
}

import { isNativeReviewProbeConfig } from "./nativeReviewConfig.ts";

test("native review probe requires a bounded nonzero port", () => {
  assert.equal(
    isNativeReviewProbeConfig("http://127.0.0.1:3030/snapshot", "token"),
    true,
  );
  for (const url of [
    "http://127.0.0.1:0/snapshot",
    "http://127.0.0.1/snapshot",
    "http://127.0.0.1:65536/snapshot",
  ]) {
    assert.equal(isNativeReviewProbeConfig(url, "token"), false, url);
  }
});
