import assert from "node:assert/strict";
import { test } from "node:test";

import { formatDownloadProgress } from "./formatDownloadProgress.mjs";

const MB = 1024 * 1024;

test("a known total gives megabytes and a percentage", () => {
  assert.equal(
    formatDownloadProgress(10 * MB, 50 * MB),
    "Downloading update — 10.0 MB of 50.0 MB (20%)",
  );
});

test("an unknown total still reports what has arrived", () => {
  // No Content-Length: a percentage would be invented, but the byte count is
  // real and is what tells the user it has not stalled.
  assert.equal(
    formatDownloadProgress(8 * MB, null),
    "Downloading update — 8.0 MB",
  );
  assert.equal(
    formatDownloadProgress(8 * MB, 0),
    "Downloading update — 8.0 MB",
  );
});

test("megabytes round rather than truncate", () => {
  assert.equal(
    formatDownloadProgress(8.25 * MB, null),
    "Downloading update — 8.3 MB",
  );
});

test("the percentage is clamped at 100", () => {
  // The last chunk can overshoot a slightly stale total, and "101%"
  // undermines confidence in everything else on the page.
  assert.equal(
    formatDownloadProgress(51 * MB, 50 * MB),
    "Downloading update — 51.0 MB of 50.0 MB (100%)",
  );
});

test("the start of a download reads as 0%, not as an error", () => {
  assert.equal(
    formatDownloadProgress(0, 50 * MB),
    "Downloading update — 0.0 MB of 50.0 MB (0%)",
  );
});

test("negative or missing byte counts do not produce nonsense", () => {
  assert.equal(
    formatDownloadProgress(-1, 50 * MB),
    "Downloading update — 0.0 MB of 50.0 MB (0%)",
  );
  assert.equal(
    formatDownloadProgress(undefined, null),
    "Downloading update — 0.0 MB",
  );
});
