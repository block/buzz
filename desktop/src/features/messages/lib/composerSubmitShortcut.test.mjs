import assert from "node:assert/strict";
import test from "node:test";

const storageKey = "buzz:composer-submit-shortcut:v1";

function createStorage(seed = {}) {
  const values = new Map(Object.entries(seed));
  return {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, String(value)),
  };
}

let loadSequence = 0;

async function loadStore(seed) {
  globalThis.window = { localStorage: createStorage(seed) };
  loadSequence += 1;
  return import(
    `./composerSubmitShortcut.ts?test=${Date.now()}-${loadSequence}`
  );
}

test("composer submit shortcut defaults to Enter", async () => {
  const store = await loadStore();
  assert.equal(store.getComposerSubmitShortcut(), "enter");
});

test("composer submit shortcut loads persisted Mod+Enter preference", async () => {
  const store = await loadStore({ [storageKey]: "mod-enter" });
  assert.equal(store.getComposerSubmitShortcut(), "mod-enter");
});

test("composer submit shortcut ignores invalid persisted values", async () => {
  const store = await loadStore({ [storageKey]: "spacebar" });
  assert.equal(store.getComposerSubmitShortcut(), "enter");
});

test("composer submit shortcut persists changes", async () => {
  const store = await loadStore();

  store.setComposerSubmitShortcut("mod-enter");

  assert.equal(store.getComposerSubmitShortcut(), "mod-enter");
  assert.equal(window.localStorage.getItem(storageKey), "mod-enter");
});
