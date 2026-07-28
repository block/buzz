import assert from "node:assert/strict";
import test from "node:test";

import { shouldUseNativeNotification } from "./desktop.ts";

test("targeted macOS Tauri notifications use the native path", () => {
  assert.equal(shouldUseNativeNotification(true, false, true, true), true);
});

test("untargeted macOS Tauri notifications use the plugin path", () => {
  assert.equal(shouldUseNativeNotification(true, false, true, false), false);
});

test("Linux Tauri notifications use the native path", () => {
  assert.equal(shouldUseNativeNotification(true, true, false, false), true);
});

test("browser notifications never use the native path", () => {
  assert.equal(shouldUseNativeNotification(false, true, true, true), false);
});

test("Windows and other Tauri notifications use the plugin path", () => {
  assert.equal(shouldUseNativeNotification(true, false, false, true), false);
});
