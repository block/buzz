import assert from "node:assert/strict";
import { afterEach, before, test } from "node:test";

import { installLocalStorage } from "../../playground/lib/testStorage.mjs";

before(() => {
  installLocalStorage();
});

afterEach(async () => {
  const { resetEmbeddedWindowsForTests } = await import("./embeddedWindows.ts");
  const { resetPopoutSettingsForTests } = await import("./popoutSettings.ts");
  resetEmbeddedWindowsForTests();
  resetPopoutSettingsForTests();
  globalThis.localStorage?.clear();
});

test("opening the same label focuses the existing embedded window", async () => {
  const {
    closeEmbeddedWindow,
    getActiveEmbeddedWindow,
    listEmbeddedWindows,
    openEmbeddedWindow,
    setEmbedInMain,
    setShowWindowsSection,
  } = await importPair();

  setShowWindowsSection(true);
  setEmbedInMain(true);
  const first = openEmbeddedWindow({
    label: "popout-thread-aaa",
    payload: { kind: "thread", title: "Design", channelId: "c1", threadId: "t1" },
  });
  const second = openEmbeddedWindow({
    label: "popout-thread-aaa",
    payload: { kind: "thread", title: "Design", channelId: "c1", threadId: "t1" },
  });
  assert.equal(listEmbeddedWindows().length, 1);
  assert.equal(first.label, second.label);
  assert.equal(getActiveEmbeddedWindow()?.label, "popout-thread-aaa");

  closeEmbeddedWindow("popout-thread-aaa");
  assert.equal(listEmbeddedWindows().length, 0);
  assert.equal(getActiveEmbeddedWindow(), null);
});

test("active embed is gated off when show-windows is off", async () => {
  const {
    getActiveEmbeddedWindow,
    openEmbeddedWindow,
    setEmbedInMain,
    setShowWindowsSection,
  } = await importPair();

  setShowWindowsSection(true);
  setEmbedInMain(true);
  openEmbeddedWindow({
    label: "popout-playground-bbb",
    payload: {
      kind: "playground",
      title: "Demo",
      playground: {
        hula: "playground",
        v: 1,
        name: "Demo",
        url: "https://app.example.com",
        sid: "demo-1",
      },
    },
  });
  assert.equal(getActiveEmbeddedWindow()?.label, "popout-playground-bbb");
  setShowWindowsSection(false);
  assert.equal(getActiveEmbeddedWindow(), null);
});

async function importPair() {
  const settings = await import("./popoutSettings.ts");
  const windows = await import("./embeddedWindows.ts");
  return { ...settings, ...windows };
}

test("dismissing and closing an embed split parks the playground host", async () => {
  const {
    closeEmbeddedWindow,
    dismissEmbeddedWindow,
    getActiveEmbeddedWindow,
    openEmbeddedWindow,
    setEmbedInMain,
    setShowWindowsSection,
    showEmbeddedWindow,
  } = await importPair();

  setShowWindowsSection(true);
  setEmbedInMain(true);
  const playground = {
    hula: "playground",
    v: 1,
    name: "Demo",
    url: "https://app.example.com",
    sid: "demo-1",
  };
  openEmbeddedWindow({
    label: "popout-split-aaa",
    payload: {
      kind: "split",
      title: "Split Demo",
      threadId: "t1",
      playground,
    },
  });
  assert.equal(getActiveEmbeddedWindow()?.label, "popout-split-aaa");
  dismissEmbeddedWindow();
  assert.equal(getActiveEmbeddedWindow(), null);

  showEmbeddedWindow("popout-split-aaa");
  assert.equal(getActiveEmbeddedWindow()?.label, "popout-split-aaa");
  openEmbeddedWindow({
    label: "popout-thread-bbb",
    payload: { kind: "thread", title: "Design", threadId: "t2" },
  });
  assert.equal(getActiveEmbeddedWindow()?.label, "popout-thread-bbb");

  closeEmbeddedWindow("popout-thread-bbb");
  assert.equal(getActiveEmbeddedWindow(), null);
  closeEmbeddedWindow("popout-split-aaa");
});


test("closeEmbeddedWindow removes inactive rows and parks active sid", async () => {
  const settings = await import("./popoutSettings.ts");
  const {
    closeEmbeddedWindow,
    getActiveEmbeddedWindow,
    listEmbeddedWindows,
    openEmbeddedWindow,
    showEmbeddedWindow,
  } = await import("./embeddedWindows.ts");

  settings.setShowWindowsSection(true);
  settings.setEmbedInMain(true);
  openEmbeddedWindow({
    label: "popout-playground-one",
    payload: {
      kind: "playground",
      title: "One",
      playground: {
        hula: "playground",
        v: 1,
        name: "One",
        url: "https://one.example.com",
        sid: "one",
      },
    },
  });
  openEmbeddedWindow({
    label: "popout-playground-two",
    payload: {
      kind: "playground",
      title: "Two",
      playground: {
        hula: "playground",
        v: 1,
        name: "Two",
        url: "https://two.example.com",
        sid: "two",
      },
    },
  });
  showEmbeddedWindow("popout-playground-two");
  assert.equal(getActiveEmbeddedWindow()?.label, "popout-playground-two");
  closeEmbeddedWindow("popout-playground-one");
  assert.equal(listEmbeddedWindows().length, 1);
  assert.equal(listEmbeddedWindows()[0]?.label, "popout-playground-two");
  assert.equal(getActiveEmbeddedWindow()?.label, "popout-playground-two");
  closeEmbeddedWindow("popout-playground-two");
  assert.equal(listEmbeddedWindows().length, 0);
  assert.equal(getActiveEmbeddedWindow(), null);
});
