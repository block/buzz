import assert from "node:assert/strict";
import test from "node:test";

import { nextInstallOutputLine } from "./useInstallOutputLine.ts";

function event(runtimeId, seq, line) {
  return { runtime_id: runtimeId, seq, line };
}

test("nextInstallOutputLine: adopts the first line for the watched runtime", () => {
  assert.deepEqual(
    nextInstallOutputLine(null, event("goose", 0, "downloading"), "goose"),
    { seq: 0, line: "downloading" },
  );
});

test("nextInstallOutputLine: a later line replaces the current one", () => {
  const current = { seq: 4, line: "downloading" };

  assert.deepEqual(
    nextInstallOutputLine(current, event("goose", 5, "unpacking"), "goose"),
    { seq: 5, line: "unpacking" },
  );
});

test("nextInstallOutputLine: ignores a line from another runtime", () => {
  const current = { seq: 1, line: "downloading" };

  assert.equal(
    nextInstallOutputLine(current, event("codex", 2, "other work"), "goose"),
    current,
  );
});

test("nextInstallOutputLine: ignores an out-of-order line", () => {
  const current = { seq: 7, line: "retrying" };

  assert.equal(
    nextInstallOutputLine(current, event("goose", 6, "stale line"), "goose"),
    current,
  );
});

test("nextInstallOutputLine: ignores a replay of the current sequence number", () => {
  const current = { seq: 7, line: "retrying" };

  assert.equal(
    nextInstallOutputLine(current, event("goose", 7, "duplicate"), "goose"),
    current,
  );
});

test("nextInstallOutputLine: a null line clears the display", () => {
  const current = { seq: 3, line: "download failed" };

  assert.deepEqual(
    nextInstallOutputLine(current, event("goose", 4, null), "goose"),
    { seq: 4, line: null },
  );
});

test("nextInstallOutputLine: a later step's first line is adopted after a higher attempt", () => {
  // The seq is install-wide: step 2 attempt 1 always follows step 1 attempt 2,
  // which is exactly what an attempt-keyed comparison got wrong.
  const current = { seq: 9, line: "step one, attempt two" };

  assert.deepEqual(
    nextInstallOutputLine(
      current,
      event("goose", 10, "step two, attempt one"),
      "goose",
    ),
    { seq: 10, line: "step two, attempt one" },
  );
});

test("nextInstallOutputLine: a first event mid-install is adopted", () => {
  assert.deepEqual(
    nextInstallOutputLine(null, event("goose", 42, "downloading"), "goose"),
    { seq: 42, line: "downloading" },
  );
});
