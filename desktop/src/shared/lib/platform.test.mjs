import assert from "node:assert/strict";
import test from "node:test";

// Save the original navigator so we can restore it after each test.
const originalNavigator = globalThis.navigator;

function withNavigator(platform, userAgent) {
  Object.defineProperty(globalThis, "navigator", {
    value: { platform, userAgent },
    configurable: true,
  });
}

function restoreNavigator() {
  Object.defineProperty(globalThis, "navigator", {
    value: originalNavigator,
    configurable: true,
  });
}

const { isMacPlatform, isLinuxPlatform, isWindowsPlatform } = await import(
  "./platform.ts"
);

// ── isWindowsPlatform ──────────────────────────────────────────────────────

test("isWindowsPlatform returns true for Win32", () => {
  withNavigator("Win32", "Mozilla/5.0");
  assert.equal(isWindowsPlatform(), true);
  restoreNavigator();
});

test("isWindowsPlatform returns true for Win64", () => {
  withNavigator("Win64", "Mozilla/5.0");
  assert.equal(isWindowsPlatform(), true);
  restoreNavigator();
});

test("isWindowsPlatform returns false for macOS", () => {
  withNavigator("MacIntel", "Mozilla/5.0");
  assert.equal(isWindowsPlatform(), false);
  restoreNavigator();
});

test("isWindowsPlatform returns false for Linux", () => {
  withNavigator("Linux x86_64", "Mozilla/5.0");
  assert.equal(isWindowsPlatform(), false);
  restoreNavigator();
});

test("isWindowsPlatform returns false when navigator is undefined", () => {
  Object.defineProperty(globalThis, "navigator", {
    value: undefined,
    configurable: true,
  });
  assert.equal(isWindowsPlatform(), false);
  restoreNavigator();
});

// ── isMacPlatform ──────────────────────────────────────────────────────────

test("isMacPlatform returns true for MacIntel", () => {
  withNavigator("MacIntel", "Mozilla/5.0");
  assert.equal(isMacPlatform(), true);
  restoreNavigator();
});

test("isMacPlatform returns false for Win32", () => {
  withNavigator("Win32", "Mozilla/5.0");
  assert.equal(isMacPlatform(), false);
  restoreNavigator();
});

// ── isLinuxPlatform ────────────────────────────────────────────────────────

test("isLinuxPlatform returns true for Linux", () => {
  withNavigator("Linux x86_64", "Mozilla/5.0");
  assert.equal(isLinuxPlatform(), true);
  restoreNavigator();
});

test("isLinuxPlatform returns false for Android", () => {
  withNavigator("Linux armv81", "Mozilla/5.0 (Linux; Android 14)");
  assert.equal(isLinuxPlatform(), false);
  restoreNavigator();
});

test("isLinuxPlatform returns false for Win32", () => {
  withNavigator("Win32", "Mozilla/5.0");
  assert.equal(isLinuxPlatform(), false);
  restoreNavigator();
});
