import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";
import { JSDOM } from "jsdom";

import {
  inspectPlaygroundWebview,
  isPlaygroundInspectTarget,
  playgroundHideCloseIsWindowScoped,
  playgroundWebviewId,
  playgroundWebviewLabelForWindow,
} from "./webview.ts";

const playgroundDom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

before(() => {
  Object.assign(globalThis, {
    document: playgroundDom.window.document,
    window: playgroundDom.window,
  });
});

afterEach(() => {
  delete globalThis.isTauri;
  delete playgroundDom.window.isTauri;
  delete globalThis.__TAURI_INTERNALS__;
  delete playgroundDom.window.__TAURI_INTERNALS__;
});

after(() => playgroundDom.window.close());

function installPlaygroundInvoke(handler) {
  const internals = { invoke: handler };
  globalThis.isTauri = true;
  playgroundDom.window.isTauri = true;
  globalThis.__TAURI_INTERNALS__ = internals;
  playgroundDom.window.__TAURI_INTERNALS__ = internals;
  return internals;
}

test("inspect targets playground-{sid} and never main", async () => {
  assert.equal(playgroundWebviewId("demo-1"), "playground-demo-1");
  assert.equal(isPlaygroundInspectTarget("playground-demo-1"), true);
  assert.equal(isPlaygroundInspectTarget("main"), false);
  const result = await inspectPlaygroundWebview("demo-1");
  assert.equal(result.webviewId, "playground-demo-1");
  assert.notEqual(result.webviewId, "main");
});

test("inspect result does not describe a resized stage or the app webview", async () => {
  const result = await inspectPlaygroundWebview("stage-keep");
  assert.equal(result.webviewId, "playground-stage-keep");
  assert.equal(isPlaygroundInspectTarget(result.webviewId), true);
  assert.equal("width" in result, false);
  assert.equal("height" in result, false);
  assert.equal(result.webviewId.startsWith("playground-"), true);
});

test("window-scoped labels stay playground-sid on main and suffix elsewhere", () => {
  assert.equal(playgroundWebviewLabelForWindow("demo-1"), "playground-demo-1");
  assert.equal(
    playgroundWebviewLabelForWindow("demo-1", "main"),
    "playground-demo-1",
  );
  assert.equal(
    playgroundWebviewLabelForWindow("demo-1", "popout-split-abc"),
    "playground-demo-1--popout-split-abc",
  );
  assert.equal(
    playgroundHideCloseIsWindowScoped(
      { sid: "demo-1", windowLabel: "main" },
      "main",
    ),
    true,
  );
  assert.equal(
    playgroundHideCloseIsWindowScoped(
      { sid: "demo-1", windowLabel: "main" },
      "popout-split-abc",
    ),
    false,
  );
});

test("late keeper visible:false show restores after stomping a visible stage show", async () => {
  const calls = [];
  let resolveKeeper;
  const keeperGate = new Promise((resolve) => {
    resolveKeeper = resolve;
  });
  let showCount = 0;
  installPlaygroundInvoke((cmd, args) => {
    calls.push({ cmd, args });
    if (cmd === "playground_webview_show") {
      showCount += 1;
      if (showCount === 1 && args.visible === false) {
        return keeperGate.then(() => ({
          sid: args.sid,
          canGoBack: false,
          canGoForward: false,
          currentUrl: args.url,
        }));
      }
      return Promise.resolve({
        sid: args.sid,
        canGoBack: false,
        canGoForward: false,
        currentUrl: args.url,
      });
    }
    return Promise.resolve(undefined);
  });

  const {
    getPlaygroundShowGeneration,
    PLAYGROUND_WEBVIEW_RESTORE_EVENT,
    resetPlaygroundShowGeneration,
    showPlaygroundWebview,
  } = await import("./webview.ts");

  resetPlaygroundShowGeneration();
  let restores = 0;
  const onRestore = () => {
    restores += 1;
  };
  window.addEventListener(PLAYGROUND_WEBVIEW_RESTORE_EVENT, onRestore);

  const keeper = showPlaygroundWebview({
    sid: "stage-1",
    url: "https://play.example",
    bounds: { x: -64, y: -64, width: 64, height: 64 },
    visible: false,
  });
  await showPlaygroundWebview({
    sid: "stage-1",
    url: "https://play.example",
    bounds: { x: 40, y: 80, width: 800, height: 600 },
    visible: true,
  });
  assert.equal(getPlaygroundShowGeneration("stage-1"), 1);
  assert.equal(restores, 0);

  resolveKeeper();
  await keeper;
  assert.equal(restores, 1, "stomped keeper must ask the stage to re-show");
  assert.ok(calls.some((row) => row.args?.visible === true));

  window.removeEventListener(PLAYGROUND_WEBVIEW_RESTORE_EVENT, onRestore);
});

test("keeper visible:false alone does not dispatch restore", async () => {
  installPlaygroundInvoke((cmd, args) => {
    if (cmd === "playground_webview_show") {
      return Promise.resolve({
        sid: args.sid,
        canGoBack: false,
        canGoForward: false,
        currentUrl: args.url,
      });
    }
    return Promise.resolve(undefined);
  });

  const {
    PLAYGROUND_WEBVIEW_RESTORE_EVENT,
    resetPlaygroundShowGeneration,
    showPlaygroundWebview,
  } = await import("./webview.ts");

  resetPlaygroundShowGeneration();
  let restores = 0;
  const onRestore = () => {
    restores += 1;
  };
  window.addEventListener(PLAYGROUND_WEBVIEW_RESTORE_EVENT, onRestore);

  await showPlaygroundWebview({
    sid: "warm-only",
    url: "https://play.example",
    bounds: { x: -64, y: -64, width: 64, height: 64 },
    visible: false,
  });
  assert.equal(restores, 0);

  window.removeEventListener(PLAYGROUND_WEBVIEW_RESTORE_EVENT, onRestore);
});
