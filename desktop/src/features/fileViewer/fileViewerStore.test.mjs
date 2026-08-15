import assert from "node:assert/strict";
import { beforeEach, test } from "node:test";

import {
  activateFileViewerTab,
  closeFileViewer,
  closeFileViewerTab,
  getFileViewerState,
  hasFileViewerHost,
  openFileViewerTab,
  registerFileViewerHost,
  resetFileViewerStore,
  subscribeFileViewer,
} from "./fileViewerStore.ts";

const TAB_A = {
  filename: "README.md",
  mime: "text/markdown",
  url: "https://r/media/a.md",
};
const TAB_B = {
  filename: "apply.sh",
  mime: "application/octet-stream",
  url: "https://r/media/b.sh",
};
const TAB_C = { filename: "notes.txt", url: "https://r/media/c.txt" };

beforeEach(() => {
  resetFileViewerStore();
});

test("openFileViewerTab adds a tab, activates it, and opens the panel", () => {
  openFileViewerTab(TAB_A);
  const state = getFileViewerState();
  assert.equal(state.isOpen, true);
  assert.equal(state.activeUrl, TAB_A.url);
  assert.deepEqual(state.tabs, [TAB_A]);
});

test("re-opening the same URL activates the existing tab without duplicating", () => {
  openFileViewerTab(TAB_A);
  openFileViewerTab(TAB_B);
  openFileViewerTab({ ...TAB_A, filename: "renamed.md" });
  const state = getFileViewerState();
  assert.equal(state.tabs.length, 2);
  assert.equal(state.activeUrl, TAB_A.url);
  // Original tab metadata wins — the URL is content-addressed.
  assert.equal(state.tabs[0].filename, "README.md");
});

test("activateFileViewerTab switches only to a known tab", () => {
  openFileViewerTab(TAB_A);
  openFileViewerTab(TAB_B);
  activateFileViewerTab(TAB_A.url);
  assert.equal(getFileViewerState().activeUrl, TAB_A.url);
  activateFileViewerTab("https://r/media/unknown.bin");
  assert.equal(getFileViewerState().activeUrl, TAB_A.url);
});

test("closing the active tab activates its right neighbor, else the left one", () => {
  openFileViewerTab(TAB_A);
  openFileViewerTab(TAB_B);
  openFileViewerTab(TAB_C);
  activateFileViewerTab(TAB_B.url);
  closeFileViewerTab(TAB_B.url);
  assert.equal(getFileViewerState().activeUrl, TAB_C.url);
  closeFileViewerTab(TAB_C.url);
  assert.equal(getFileViewerState().activeUrl, TAB_A.url);
});

test("closing an inactive tab keeps the active tab", () => {
  openFileViewerTab(TAB_A);
  openFileViewerTab(TAB_B);
  closeFileViewerTab(TAB_A.url);
  const state = getFileViewerState();
  assert.equal(state.activeUrl, TAB_B.url);
  assert.deepEqual(state.tabs, [TAB_B]);
});

test("closing the last tab closes the panel", () => {
  openFileViewerTab(TAB_A);
  closeFileViewerTab(TAB_A.url);
  const state = getFileViewerState();
  assert.equal(state.isOpen, false);
  assert.equal(state.activeUrl, null);
  assert.deepEqual(state.tabs, []);
});

test("closeFileViewer hides the panel but keeps tabs for a later reopen", () => {
  openFileViewerTab(TAB_A);
  openFileViewerTab(TAB_B);
  closeFileViewer();
  const closed = getFileViewerState();
  assert.equal(closed.isOpen, false);
  assert.equal(closed.tabs.length, 2);
  openFileViewerTab(TAB_A);
  const reopened = getFileViewerState();
  assert.equal(reopened.isOpen, true);
  assert.equal(reopened.tabs.length, 2);
});

test("resetFileViewerStore drops all tabs (community switch)", () => {
  openFileViewerTab(TAB_A);
  resetFileViewerStore();
  const state = getFileViewerState();
  assert.deepEqual(state, { activeUrl: null, isOpen: false, tabs: [] });
});

test("subscribers are notified on every state change", () => {
  let calls = 0;
  const unsubscribe = subscribeFileViewer(() => {
    calls += 1;
  });
  openFileViewerTab(TAB_A);
  closeFileViewer();
  unsubscribe();
  closeFileViewerTab(TAB_A.url);
  assert.equal(calls, 2);
});

test("host registration reports availability and unwinds on cleanup", () => {
  assert.equal(hasFileViewerHost(), false);
  const unregisterFirst = registerFileViewerHost();
  const unregisterSecond = registerFileViewerHost();
  assert.equal(hasFileViewerHost(), true);
  unregisterFirst();
  assert.equal(hasFileViewerHost(), true);
  unregisterSecond();
  assert.equal(hasFileViewerHost(), false);
});
