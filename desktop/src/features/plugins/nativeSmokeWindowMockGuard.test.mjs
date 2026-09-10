import assert from "node:assert/strict";
import { after, before, test } from "node:test";

/**
 * Regression for the same defect class as `nativeSmokeInvokeBridge.test.mjs`,
 * one level earlier in startup: `@tauri-apps/api/mocks`' `mockWindows` does
 * `window.__TAURI_INTERNALS__.metadata = {...}` (mocks.js:209), a plain
 * assignment. The installed Tauri runtime defines that same `metadata`
 * property via `Object.defineProperty` (`webview.rs`) with no `writable`, no
 * `configurable` — so calling `mockWindows` against a real native window
 * throws the identical class of strict-mode TypeError the invoke wrapper
 * fix works around, and it runs *before* that fix's own call site
 * (`e2eBridge.ts`'s `maybeInstallE2eTauriMocks` calls `mockWindows`
 * unconditionally, ahead of the native-smoke `invoke` wrapper install).
 *
 * The fix is a guard in `e2eBridge.ts` (`if (!nativeSmoke) mockWindows(...)`)
 * — a native-smoke run must never call it at all, both to avoid this throw
 * and because doing so would replace the real native window/webview metadata
 * with fake data. This test reproduces the throw directly against the real
 * `mockWindows` function with a faithfully frozen `metadata` descriptor, to
 * prove *why* skipping the call (rather than trying to make it succeed) is
 * the only correct fix.
 */

let originalWindow;

before(() => {
  originalWindow = globalThis.window;
});

after(() => {
  globalThis.window = originalWindow;
});

function makeWindowWithFrozenMetadata() {
  const internals = {};
  Object.defineProperty(internals, "metadata", {
    value: { currentWindow: { label: "main" } },
    // No `writable`, no `configurable` — matches installed tauri-2.11.5's
    // src/manager/webview.rs, which defines `metadata` the same way it
    // defines `invoke`.
  });
  return { __TAURI_INTERNALS__: internals };
}

test("reproduces the defect: mockWindows throws against a real native metadata descriptor", async () => {
  globalThis.window = makeWindowWithFrozenMetadata();
  const { mockWindows } = await import("@tauri-apps/api/mocks");

  assert.throws(() => mockWindows("main"), TypeError);
});

test("e2eBridge.ts guards the mockWindows call site behind !nativeSmoke", async () => {
  // `maybeInstallE2eTauriMocks` is a ~15k-line function with heavy runtime
  // dependencies (relay client, IndexedDB, …) that make invoking it directly
  // in a unit test impractical. The call-site guard itself is small and
  // load-bearing enough to check structurally: `mockWindows` must never be
  // reachable when `nativeSmoke` is true, given the throw reproduced above.
  const { readFile } = await import("node:fs/promises");
  const source = await readFile(
    new URL("../../testing/e2eBridge.ts", import.meta.url),
    "utf8",
  );
  const guarded = /if\s*\(\s*!nativeSmoke\s*\)\s*\{[^}]*mockWindows\(/s;
  assert.match(
    source,
    guarded,
    "mockWindows(...) must be called only inside an `if (!nativeSmoke)` guard",
  );
});
