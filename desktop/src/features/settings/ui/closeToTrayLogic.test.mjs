import test from "node:test";
import assert from "node:assert/strict";

import {
  CLOSE_TO_TRAY_DEFAULT,
  CLOSE_TO_TRAY_OPTIONS,
  isCloseToTrayBehavior,
  loadCloseToTrayBehavior,
} from "./closeToTrayLogic.ts";

test("default is keepRunning and is a valid behavior", () => {
  assert.equal(CLOSE_TO_TRAY_DEFAULT, "keepRunning");
  assert.equal(isCloseToTrayBehavior(CLOSE_TO_TRAY_DEFAULT), true);
});

test("accepts the three documented behaviors", () => {
  assert.equal(isCloseToTrayBehavior("keepRunning"), true);
  assert.equal(isCloseToTrayBehavior("minimizeToTray"), true);
  assert.equal(isCloseToTrayBehavior("quitWhenClosed"), true);
});

test("rejects unknown and non-string values", () => {
  for (const value of ["", "quit", "hide", null, undefined, 0, {}, []]) {
    assert.equal(isCloseToTrayBehavior(value), false);
  }
});

test("every option has a unique valid behavior with copy", () => {
  const values = new Set();
  for (const option of CLOSE_TO_TRAY_OPTIONS) {
    assert.equal(isCloseToTrayBehavior(option.value), true);
    assert.equal(values.has(option.value), false);
    values.add(option.value);
    assert.ok(option.label.length > 0);
    assert.ok(option.description.length > 0);
  }
  assert.equal(values.size, CLOSE_TO_TRAY_OPTIONS.length);
});

test("load falls back to the default when Tauri is unavailable", async () => {
  // In plain node there is no Tauri bridge, so load resolves to the default.
  assert.equal(await loadCloseToTrayBehavior(), CLOSE_TO_TRAY_DEFAULT);
});
