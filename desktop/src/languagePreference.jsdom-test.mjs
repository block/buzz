import assert from "node:assert/strict";
import { afterEach, test } from "node:test";
import React from "react";
import { act, cleanup, fireEvent, render } from "@testing-library/react";
import { LanguageSetting } from "./features/settings/ui/LanguageSetting.tsx";
import {
  getLanguagePreferenceState,
  i18n,
  LANGUAGE_STORAGE_KEY,
  setAppLanguage,
  subscribeLanguagePreference,
} from "./i18n.ts";

afterEach(async () => {
  cleanup();
  delete globalThis.isTauri;
  delete window.__TAURI_INTERNALS__;
  await setAppLanguage("en");
  localStorage.clear();
});

test("overlapping selections serialize native menus and only persist the latest choice", async () => {
  await setAppLanguage("en");
  const started = Promise.withResolvers();
  const native = Promise.withResolvers();
  const calls = [];
  const published = [];
  const unsubscribe = subscribeLanguagePreference(() => {
    published.push(getLanguagePreferenceState().preference);
  });
  globalThis.isTauri = true;
  window.__TAURI_INTERNALS__ = {
    invoke(command, args) {
      assert.equal(command, "set_desktop_language");
      calls.push(args.language);
      if (args.language === "ko") {
        started.resolve();
        return native.promise;
      }
      return Promise.resolve();
    },
  };
  const first = setAppLanguage("ko");
  await started.promise;
  const latest = setAppLanguage("en");
  try {
    await new Promise(setImmediate);
    assert.deepEqual(calls, ["ko"], "do not race two native menu updates");
  } finally {
    native.resolve();
    await Promise.all([first, latest]);
    unsubscribe();
  }
  assert.deepEqual(calls, ["ko", "en"]);
  assert.deepEqual(
    published,
    ["en"],
    "a retired request cannot publish stale state",
  );
  assert.equal(getLanguagePreferenceState().preference, "en");
  assert.equal(localStorage.getItem(LANGUAGE_STORAGE_KEY), "en");
  assert.equal(document.documentElement.lang, "en");
  assert.equal(i18n.resolvedLanguage, "en");
});

test("another window's stored selection applies without writing it back", async () => {
  await setAppLanguage("en");
  localStorage.setItem(LANGUAGE_STORAGE_KEY, "ko");
  const prototype = Object.getPrototypeOf(localStorage);
  const original = prototype.setItem;
  let writes = 0;
  prototype.setItem = function (...args) {
    writes += 1;
    return original.apply(this, args);
  };
  try {
    window.dispatchEvent(
      new window.StorageEvent("storage", {
        key: LANGUAGE_STORAGE_KEY,
        newValue: "ko",
        storageArea: localStorage,
      }),
    );
    await new Promise(setImmediate);
    assert.equal(i18n.resolvedLanguage, "ko");
    assert.equal(getLanguagePreferenceState().preference, "ko");
    assert.equal(document.documentElement.lang, "ko");
    assert.equal(writes, 0);
  } finally {
    prototype.setItem = original;
  }
});

test("a failed language load does not poison the next request", async () => {
  await setAppLanguage("en");
  const original = i18n.changeLanguage;
  i18n.changeLanguage = () =>
    Promise.reject(new Error("catalogue unavailable"));
  try {
    await assert.rejects(setAppLanguage("ko"), /catalogue unavailable/);
  } finally {
    i18n.changeLanguage = original;
  }
  await setAppLanguage("ko");
  assert.equal(i18n.resolvedLanguage, "ko");
  assert.equal(getLanguagePreferenceState().preference, "ko");
});

test("a failed selection remains visible after reopening Settings and clears on retry", async () => {
  await setAppLanguage("en");
  const original = i18n.changeLanguage;
  i18n.changeLanguage = () =>
    Promise.reject(new Error("catalogue unavailable"));
  const screen = render(React.createElement(LanguageSetting));
  try {
    await act(async () => {
      fireEvent.change(screen.getByTestId("app-language"), {
        target: { value: "ko" },
      });
    });
    assert.equal(
      screen.getByRole("alert").textContent,
      "The language could not be changed. Try again.",
    );
    screen.unmount();
    const reopened = render(React.createElement(LanguageSetting));
    assert.ok(reopened.getByRole("alert"));
    i18n.changeLanguage = original;
    await act(async () => {
      fireEvent.click(reopened.getByRole("button", { name: "Try again" }));
    });
    assert.equal(reopened.queryByRole("alert"), null);
    assert.equal(reopened.getByTestId("app-language").value, "ko");
  } finally {
    i18n.changeLanguage = original;
  }
});

test("native menu failures remain retryable without losing the selected language", async () => {
  await setAppLanguage("en");
  globalThis.isTauri = true;
  window.__TAURI_INTERNALS__ = {
    invoke: () => Promise.reject(new Error("menu unavailable")),
  };
  const first = await setAppLanguage("ko");
  assert.deepEqual(first, { saved: true, nativeSynced: false });
  assert.equal(getLanguagePreferenceState().preference, "ko");
  assert.equal(i18n.resolvedLanguage, "ko");
  const screen = render(React.createElement(LanguageSetting));
  assert.equal(screen.getByTestId("app-language").value, "ko");
  assert.ok(screen.getByRole("alert"));
  window.__TAURI_INTERNALS__.invoke = () => Promise.resolve();
  await act(async () =>
    fireEvent.click(screen.getByTestId("app-language-retry")),
  );
  assert.equal(screen.queryByRole("alert"), null);
  assert.equal(getLanguagePreferenceState().nativeSynced, true);
});
