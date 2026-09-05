import assert from "node:assert/strict";
import { test } from "node:test";

// Standalone-web coverage for the isTauri() guards in deep-link.ts.
//
// This file deliberately sets no Tauri globals at all. `isTauri()` reads
// `globalThis.isTauri` (it ignores `__TAURI_INTERNALS__`), so leaving the
// global unset is exactly the browser runtime these guards exist for — no
// mocking required, and the real `@tauri-apps/api` modules are under test
// rather than a stand-in for them.
//
// This matters because deep-link.test.mjs sets `globalThis.isTauri = true`,
// so every test in that file exercises the native branch only. Without this
// file the guards themselves have no coverage.
//
// If a guard ever leaks, the real `invoke()`/`listen()` reject without Tauri
// internals, so the awaits below fail rather than quietly passing.
//
// It lives in its own file because node's per-process ESM module cache would
// otherwise hand a second import of deep-link.ts the instance evaluated under
// the other file's globals. `node --test` runs each matched file in its own
// process, which sidesteps that.

const {
  listenForDeepLinks,
  resetNavigationDeepLinkDrain,
  listenForNavigationDeepLinks,
  listenForEntityDeepLinks,
  listenForNostrBindDeepLinks,
} = await import("@/shared/deep-link.ts");

function makeDeps() {
  const state = {
    onboarding: 0,
    addCommunity: 0,
    availabilitySubscriptions: 0,
  };
  return {
    state,
    deps: {
      startCommunityOnboarding: () => {
        state.onboarding += 1;
        return true;
      },
      openAddCommunity: () => {
        state.addCommunity += 1;
        return true;
      },
      onAddCommunityAvailable: () => {
        state.availabilitySubscriptions += 1;
        return () => {
          state.availabilitySubscriptions -= 1;
        };
      },
    },
  };
}

test("listenForDeepLinks resolves to a callable cleanup without reaching the native bridge", async () => {
  const harness = makeDeps();

  const unlisten = await listenForDeepLinks(harness.deps);

  assert.equal(typeof unlisten, "function");
  assert.doesNotThrow(() => unlisten());
  assert.equal(harness.state.onboarding, 0);
  assert.equal(harness.state.addCommunity, 0);
});

test("listenForDeepLinks does not subscribe to add-community availability in web mode", async () => {
  const harness = makeDeps();

  const unlisten = await listenForDeepLinks(harness.deps);

  // The pending-link queue the drain reads is native-only, so a subscription
  // here could never do work. Guarding before subscribing keeps that explicit.
  assert.equal(harness.state.availabilitySubscriptions, 0);
  unlisten();
  assert.equal(harness.state.availabilitySubscriptions, 0);
});

test("resetNavigationDeepLinkDrain resolves without clearing a native queue", async () => {
  await assert.doesNotReject(() => resetNavigationDeepLinkDrain());
});

test("listenForNavigationDeepLinks resolves to a callable cleanup and never routes", async () => {
  let routed = 0;
  const route = () => {
    routed += 1;
    return true;
  };

  const unlisten = await listenForNavigationDeepLinks(route, route);

  assert.equal(typeof unlisten, "function");
  assert.doesNotThrow(() => unlisten());
  assert.equal(routed, 0);
});

test("listenForEntityDeepLinks resolves to a callable cleanup and never routes", async () => {
  let routed = 0;

  const unlisten = await listenForEntityDeepLinks(() => {
    routed += 1;
    return true;
  });

  assert.equal(typeof unlisten, "function");
  assert.doesNotThrow(() => unlisten());
  assert.equal(routed, 0);
});

test("listenForNostrBindDeepLinks resolves to a callable cleanup and never routes", async () => {
  let routed = 0;

  const unlisten = await listenForNostrBindDeepLinks(() => {
    routed += 1;
  });

  assert.equal(typeof unlisten, "function");
  assert.doesNotThrow(() => unlisten());
  assert.equal(routed, 0);
});
