/**
 * Language picker widget tests (FR-001 / NFR-001): the Settings →
 * Appearance row renders translated labels, switches the app language
 * live, and exposes a correct accessible name.
 */
import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

function installLocalStorage(entries = {}) {
  const store = new Map(Object.entries(entries));
  const storage = {
    getItem: (key) => (store.has(key) ? store.get(key) : null),
    setItem: (key, value) => {
      store.set(key, String(value));
    },
  };
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    value: storage,
  });
  return store;
}

let React;
let rtl;
let store;

before(async () => {
  Object.assign(globalThis, {
    document: dom.window.document,
    getComputedStyle: dom.window.getComputedStyle.bind(dom.window),
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    Node: dom.window.Node,
    window: dom.window,
  });
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: { languages: ["en-US", "en"], userAgent: "buzz-unit-test" },
  });
  store = installLocalStorage();
  React = await import("react");
  rtl = await import("@testing-library/react");
  const indexModule = await import("@/i18n/index.ts");
  indexModule.initializeI18n();
});

afterEach(async () => {
  rtl.cleanup();
});

after(() => dom.window.close());

function renderLanguageSetting() {
  return rtl.render(React.createElement(LanguageSetting, null));
}

let LanguageSetting;

before(async () => {
  ({ LanguageSetting } = await import("./LanguageSetting.tsx"));
});

test("picker renders the English row and all three options", async () => {
  renderLanguageSetting();

  const screen = rtl.screen;
  assert.equal(
    screen.getByTestId("language-row").querySelector("p").textContent,
    "Language",
  );
  assert.equal(screen.getByTestId("language-system").textContent, "System");
  assert.equal(screen.getByTestId("language-en").textContent, "English");
  assert.equal(screen.getByTestId("language-zh-Hans").textContent, "简体中文");
  // The fieldset legend carries the row's accessible name.
  assert.equal(
    screen.getByTestId("language-control").querySelector("legend").textContent,
    "Language",
  );
  assert.equal(document.documentElement.lang, "en");
});

test("choosing 简体中文 applies immediately and persists", async () => {
  renderLanguageSetting();
  const { act, screen } = rtl;
  await act(async () => {
    screen.getByTestId("language-zh-Hans").click();
  });

  assert.equal(
    screen.getByTestId("language-row").querySelector("p").textContent,
    "语言",
  );
  assert.equal(screen.getByTestId("language-system").textContent, "跟随系统");
  assert.equal(
    screen.getByTestId("language-control").querySelector("legend").textContent,
    "语言",
  );
  assert.equal(document.documentElement.lang, "zh-Hans");
  assert.equal(store.get("buzz-language"), "zh-Hans");
});

test("switching to system follows the system locale", async () => {
  renderLanguageSetting();
  const { act, screen } = rtl;
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: { languages: ["fr", "zh-CN"], userAgent: "buzz-unit-test" },
  });
  await act(async () => {
    screen.getByTestId("language-system").click();
  });
  assert.equal(document.documentElement.lang, "zh-Hans");
  assert.equal(store.get("buzz-language"), "system");
});
