import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import { installLocalStorage } from "../../playground/lib/testStorage.mjs";

import {
  filterPopoutWindows,
  playgroundSidsHostedInOsPopouts,
  popoutKindFromLabel,
  resolvePopoutTitle,
  resetPopoutWindowsForTests,
} from "./popoutWindows.ts";
import { popoutStorageKey, writePopoutPayload } from "./popoutWindow.ts";

afterEach(() => {
  resetPopoutWindowsForTests();
  if (typeof globalThis.localStorage?.clear === "function") {
    globalThis.localStorage.clear();
  }
  delete globalThis.__TAURI_INTERNALS__;
  delete globalThis.isTauri;
});

test("filterPopoutWindows keeps only labels starting with popout-", () => {
  const filtered = filterPopoutWindows([
    { label: "main", title: "Buzz" },
    { label: "popout-thread-aaa", title: "Thread" },
    { label: "huddle-xyz", title: "Huddle" },
    { label: "popout-playground-bbb", title: "Demo" },
    { label: "popout-split-ccc", title: "Split Demo" },
  ]);
  assert.deepEqual(
    filtered.map((row) => row.label),
    ["popout-thread-aaa", "popout-playground-bbb", "popout-split-ccc"],
  );
});

test("filterPopoutWindows drops an empty list to empty", () => {
  assert.deepEqual(filterPopoutWindows([]), []);
});

test("popoutKindFromLabel reads kind from the label prefix", () => {
  assert.equal(popoutKindFromLabel("popout-thread-aaa"), "thread");
  assert.equal(popoutKindFromLabel("popout-playground-bbb"), "playground");
  assert.equal(popoutKindFromLabel("popout-split-ccc"), "split");
  assert.equal(popoutKindFromLabel("main"), null);
});

test("resolvePopoutTitle prefers native title, then payload, then kind", () => {
  assert.equal(
    resolvePopoutTitle({ label: "popout-thread-aaa", title: "  Hello  " }),
    "Hello",
  );
  assert.equal(
    resolvePopoutTitle({ label: "popout-thread-aaa", title: "  " }),
    "Thread",
  );
  assert.equal(
    resolvePopoutTitle({ label: "popout-split-bbb", title: "" }),
    "Split",
  );
  assert.equal(
    resolvePopoutTitle({ label: "popout-playground-ccc", title: "" }),
    "Playground",
  );
});

test("resolvePopoutTitle falls back to payload title when native is empty", () => {
  installLocalStorage();
  writePopoutPayload("popout-thread-aaa", {
    kind: "thread",
    title: "Design review",
  });
  assert.equal(
    resolvePopoutTitle({ label: "popout-thread-aaa", title: "" }),
    "Design review",
  );
});

test("writePopoutPayload stores the window title for list fallback", () => {
  installLocalStorage();
  writePopoutPayload("popout-playground-xyz", {
    kind: "playground",
    title: "Demo",
  });
  const raw = globalThis.localStorage.getItem(
    popoutStorageKey("popout-playground-xyz"),
  );
  assert.ok(raw);
  assert.equal(JSON.parse(raw).title, "Demo");
});

test("focusPopoutWindow invokes the native focus command", async () => {
  const invokes = [];
  const internals = {
    invoke(cmd, args) {
      invokes.push({ cmd, args });
      return Promise.resolve();
    },
  };
  globalThis.isTauri = true;
  globalThis.__TAURI_INTERNALS__ = internals;
  globalThis.window = globalThis.window ?? globalThis;
  globalThis.window.__TAURI_INTERNALS__ = internals;
  const { focusPopoutWindow } = await import("./popoutWindows.ts");
  await focusPopoutWindow("popout-thread-aaa");
  assert.deepEqual(invokes, [
    { cmd: "focus_popout_window", args: { label: "popout-thread-aaa" } },
  ]);
  await focusPopoutWindow("main");
  assert.equal(invokes.length, 1);
});

test("playgroundSidsHostedInOsPopouts collects split and playground sids", () => {
  installLocalStorage();
  const playground = {
    hula: "playground",
    v: 1,
    name: "Demo",
    url: "https://app.example.com",
    sid: "demo-1",
  };
  writePopoutPayload("popout-split-aaa", {
    kind: "split",
    playground,
  });
  writePopoutPayload("popout-thread-bbb", { kind: "thread", title: "Thread" });
  writePopoutPayload("popout-playground-ccc", {
    kind: "playground",
    playground: { ...playground, sid: "demo-2" },
  });
  const hosted = playgroundSidsHostedInOsPopouts([
    { label: "popout-split-aaa" },
    { label: "popout-thread-bbb" },
    { label: "popout-playground-ccc" },
    { label: "main" },
  ]);
  assert.deepEqual([...hosted].sort(), ["demo-1", "demo-2"]);
});
