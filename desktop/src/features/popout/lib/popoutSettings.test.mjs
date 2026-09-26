import assert from "node:assert/strict";
import { afterEach, before, test } from "node:test";

import { installLocalStorage } from "../../playground/lib/testStorage.mjs";

before(() => {
  installLocalStorage();
});

afterEach(async () => {
  const { resetPopoutSettingsForTests } = await import("./popoutSettings.ts");
  resetPopoutSettingsForTests();
  globalThis.localStorage?.clear();
});

test("start-fullscreen defaults off and persists", async () => {
  const { START_FULLSCREEN_KEY, isStartFullscreenEnabled, setStartFullscreen } =
    await import("./popoutSettings.ts");

  assert.equal(isStartFullscreenEnabled(), false);
  setStartFullscreen(true);
  assert.equal(isStartFullscreenEnabled(), true);
  assert.equal(globalThis.localStorage.getItem(START_FULLSCREEN_KEY), "true");
});

test("setting embed true clears fullscreen", async () => {
  const {
    EMBED_IN_MAIN_KEY,
    START_FULLSCREEN_KEY,
    getPopoutSettings,
    isEmbedInMainEnabled,
    isStartFullscreenEnabled,
    setEmbedInMain,
    setStartFullscreen,
  } = await import("./popoutSettings.ts");

  setStartFullscreen(true);
  setEmbedInMain(true);

  assert.equal(isEmbedInMainEnabled(), true);
  assert.equal(isStartFullscreenEnabled(), false);
  assert.equal(getPopoutSettings().embedInMain, true);
  assert.equal(getPopoutSettings().startFullscreen, false);
  assert.equal(globalThis.localStorage.getItem(EMBED_IN_MAIN_KEY), "true");
  assert.equal(globalThis.localStorage.getItem(START_FULLSCREEN_KEY), "false");
});

test("setting fullscreen true clears embed", async () => {
  const {
    EMBED_IN_MAIN_KEY,
    START_FULLSCREEN_KEY,
    getPopoutSettings,
    isEmbedInMainEnabled,
    isStartFullscreenEnabled,
    setEmbedInMain,
    setStartFullscreen,
  } = await import("./popoutSettings.ts");

  setEmbedInMain(true);
  setStartFullscreen(true);

  assert.equal(isStartFullscreenEnabled(), true);
  assert.equal(isEmbedInMainEnabled(), false);
  assert.equal(getPopoutSettings().embedInMain, false);
  assert.equal(getPopoutSettings().startFullscreen, true);
  assert.equal(globalThis.localStorage.getItem(EMBED_IN_MAIN_KEY), "false");
  assert.equal(globalThis.localStorage.getItem(START_FULLSCREEN_KEY), "true");
});

test("embed defaults off and persists", async () => {
  const { EMBED_IN_MAIN_KEY, isEmbedInMainEnabled, setEmbedInMain } =
    await import("./popoutSettings.ts");

  assert.equal(isEmbedInMainEnabled(), false);
  setEmbedInMain(true);
  assert.equal(isEmbedInMainEnabled(), true);
  assert.equal(globalThis.localStorage.getItem(EMBED_IN_MAIN_KEY), "true");
});
