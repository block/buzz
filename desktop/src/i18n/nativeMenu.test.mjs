/**
 * The frontend half of FR-008: Rust is told the language after i18n boots and
 * after every switch, and only ever with a value the native enum accepts.
 *
 * The bridge is faked rather than the command, because what is under test is
 * the *push* — the same call Tauri's own `invoke` would forward, with the
 * payload Rust deserializes into `AppMenuLocale`.
 */
import assert from "node:assert/strict";
import test from "node:test";

/** @type {{command: string, args: Record<string, unknown>}[]} */
const calls = [];

globalThis.window = {
  __TAURI_INTERNALS__: {
    async invoke(command, args) {
      calls.push({ command, args });
    },
  },
};
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  value: { languages: ["en-US", "en"], userAgent: "buzz-unit-test" },
});

const { initializeI18n, syncNativeMenuLocale } = await import("@/i18n");
const { setLanguagePreference } = await import("@/i18n/language");

async function waitForCallCount(target) {
  for (let attempt = 0; attempt < 200; attempt += 1) {
    if (calls.length >= target) return;
    await new Promise((resolve) => {
      setTimeout(resolve, 1);
    });
  }
  throw new Error(`timed out with ${calls.length}/${target} menu calls`);
}

test("booting i18n hands the native menu the boot language", async () => {
  initializeI18n();
  await waitForCallCount(1);
  assert.deepEqual(calls[0], {
    command: "set_app_menu_locale",
    args: { locale: "en" },
  });
});

test("every language switch re-notifies Rust", async () => {
  setLanguagePreference("zh-Hans");
  await waitForCallCount(2);
  assert.deepEqual(calls[1], {
    command: "set_app_menu_locale",
    args: { locale: "zh-Hans" },
  });
  setLanguagePreference("en");
  await waitForCallCount(3);
  assert.equal(calls[2].args.locale, "en");
});

test("a language outside the native enum is never sent", async () => {
  // Rust would reject it; the point is that no call leaves the webview.
  await syncNativeMenuLocale("fr-FR");
  await syncNativeMenuLocale("");
  await syncNativeMenuLocale("zh-Hans-CN");
  assert.equal(calls.length, 3);
});

test("a failing or absent bridge is swallowed, never surfaced to the UI", async () => {
  globalThis.window.__TAURI_INTERNALS__.invoke = async () => {
    throw new Error("command not registered");
  };
  await syncNativeMenuLocale("zh-Hans");
  assert.equal(calls.length, 3, "the rejected call must not be recorded");

  delete globalThis.window.__TAURI_INTERNALS__;
  await syncNativeMenuLocale("en");
  assert.equal(calls.length, 3);
});
