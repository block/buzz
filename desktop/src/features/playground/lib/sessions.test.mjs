import assert from "node:assert/strict";
import { afterEach, before, test } from "node:test";

import { installLocalStorage } from "./testStorage.mjs";

const card = {
  hula: "playground",
  v: 1,
  name: "Demo",
  url: "https://app.example.com",
  pin: "1234",
  sid: "demo-1",
  stack: "hula-app",
};

before(() => {
  installLocalStorage();
});

afterEach(async () => {
  const { resetPlaygroundState } = await import("./sessions.ts");
  resetPlaygroundState();
  const { resetEmbeddedWindowsForTests } = await import(
    "../../popout/lib/embeddedWindows.ts"
  );
  const { resetPopoutSettingsForTests } = await import(
    "../../popout/lib/popoutSettings.ts"
  );
  resetEmbeddedWindowsForTests();
  resetPopoutSettingsForTests();
  globalThis.localStorage?.clear();
});

test("add creates a personal session; dismiss parks; dispose removes", async () => {
  const {
    addPlaygroundSession,
    configurePlaygroundScope,
    dismissPlayground,
    disposePlayground,
    getActivePlaygroundSid,
    listPlaygroundSessions,
    showPlaygroundSession,
  } = await import("./sessions.ts");

  configurePlaygroundScope("pub", "wss://relay.example.com");
  addPlaygroundSession(card);
  assert.equal(listPlaygroundSessions().length, 1);
  assert.equal(getActivePlaygroundSid(), "demo-1");

  dismissPlayground();
  assert.equal(getActivePlaygroundSid(), null);
  assert.equal(listPlaygroundSessions()[0]?.sid, "demo-1");

  showPlaygroundSession("demo-1");
  assert.equal(getActivePlaygroundSid(), "demo-1");

  disposePlayground("demo-1");
  assert.equal(listPlaygroundSessions().length, 0);
  assert.equal(getActivePlaygroundSid(), null);

  addPlaygroundSession(card);
  assert.equal(listPlaygroundSessions().length, 1);
});

test("update chip sets on card match and clears on Open", async () => {
  const {
    addPlaygroundSession,
    configurePlaygroundScope,
    dismissPlayground,
    listPlaygroundSessions,
    markPlaygroundUpdate,
    notePlaygroundCard,
    showPlaygroundSession,
  } = await import("./sessions.ts");

  configurePlaygroundScope("pub", "wss://relay.example.com");
  addPlaygroundSession(card);
  dismissPlayground();
  markPlaygroundUpdate("demo-1");
  assert.equal(listPlaygroundSessions()[0]?.hasUpdate, true);

  showPlaygroundSession("demo-1");
  assert.equal(listPlaygroundSessions()[0]?.hasUpdate, false);

  dismissPlayground();
  notePlaygroundCard({ ...card, sid: "other-sid" });
  assert.equal(listPlaygroundSessions()[0]?.hasUpdate, true);
});

test("hasPlaygroundSession tracks left-menu rows; parkPlaygroundThen dismisses", async () => {
  const {
    addPlaygroundSession,
    configurePlaygroundScope,
    getActivePlaygroundSid,
    hasPlaygroundSession,
    parkPlaygroundThen,
  } = await import("./sessions.ts");

  configurePlaygroundScope("pub", "wss://relay.example.com");
  assert.equal(hasPlaygroundSession("demo-1"), false);
  addPlaygroundSession(card);
  assert.equal(hasPlaygroundSession("demo-1"), true);
  assert.equal(getActivePlaygroundSid(), "demo-1");

  let selected = false;
  parkPlaygroundThen(() => {
    selected = true;
  })();
  assert.equal(selected, true);
  assert.equal(getActivePlaygroundSid(), null);
  assert.equal(hasPlaygroundSession("demo-1"), true);
});

test("optional pin is omitted from the session and stays openable", async () => {
  const {
    addPlaygroundSession,
    configurePlaygroundScope,
    getActivePlaygroundSid,
    listPlaygroundSessions,
  } = await import("./sessions.ts");

  configurePlaygroundScope("pub", "wss://relay.example.com");
  const { pin: _pin, ...withoutPin } = card;
  addPlaygroundSession(withoutPin);
  assert.equal(getActivePlaygroundSid(), "demo-1");
  assert.equal(listPlaygroundSessions()[0]?.pin, undefined);
});

test("parkPlaygroundHost parks overlay and embed-only split", async () => {
  const {
    addPlaygroundSession,
    configurePlaygroundScope,
    getActivePlaygroundSid,
    parkPlaygroundHost,
    parkPlaygroundThen,
  } = await import("./sessions.ts");
  const settings = await import("../../popout/lib/popoutSettings.ts");
  const embedded = await import("../../popout/lib/embeddedWindows.ts");

  configurePlaygroundScope("pub", "wss://relay.example.com");
  addPlaygroundSession(card);
  assert.equal(getActivePlaygroundSid(), "demo-1");
  parkPlaygroundHost();
  assert.equal(getActivePlaygroundSid(), null);

  settings.setShowWindowsSection(true);
  settings.setEmbedInMain(true);
  embedded.openEmbeddedWindow({
    label: "popout-split-aaa",
    payload: {
      kind: "split",
      title: "Split Demo",
      threadId: "t1",
      playground: card,
    },
  });
  assert.equal(embedded.getActiveEmbeddedWindow()?.label, "popout-split-aaa");
  assert.equal(getActivePlaygroundSid(), null);

  let selected = false;
  parkPlaygroundThen(() => {
    selected = true;
  })();
  assert.equal(selected, true);
  assert.equal(embedded.getActiveEmbeddedWindow(), null);
});

test("dismissPlayground parks embed-only host like parkPlaygroundHost", async () => {
  const {
    configurePlaygroundScope,
    dismissPlayground,
    getActivePlaygroundSid,
  } = await import("./sessions.ts");
  const settings = await import("../../popout/lib/popoutSettings.ts");
  const embedded = await import("../../popout/lib/embeddedWindows.ts");

  configurePlaygroundScope("pub", "wss://relay.example.com");
  settings.setShowWindowsSection(true);
  settings.setEmbedInMain(true);
  embedded.openEmbeddedWindow({
    label: "popout-playground-bbb",
    payload: {
      kind: "playground",
      title: "Demo",
      playground: card,
    },
  });
  assert.equal(
    embedded.getActiveEmbeddedWindow()?.label,
    "popout-playground-bbb",
  );
  assert.equal(getActivePlaygroundSid(), null);

  dismissPlayground();
  assert.equal(embedded.getActiveEmbeddedWindow(), null);
  assert.equal(getActivePlaygroundSid(), null);
});

test("dismissPlayground is an alias of parkPlaygroundHost for chrome X", async () => {
  const {
    addPlaygroundSession,
    configurePlaygroundScope,
    dismissPlayground,
    getActivePlaygroundSid,
  } = await import("./sessions.ts");
  const settings = await import("../../popout/lib/popoutSettings.ts");
  const embedded = await import("../../popout/lib/embeddedWindows.ts");

  configurePlaygroundScope("pub", "wss://relay.example.com");
  addPlaygroundSession(card);
  settings.setShowWindowsSection(true);
  settings.setEmbedInMain(true);
  embedded.openEmbeddedWindow({
    label: "popout-split-ccc",
    payload: {
      kind: "split",
      title: "Split",
      threadId: "t1",
      playground: { ...card, sid: "demo-2", name: "Other" },
    },
  });
  // Overlay was parked when the embed opened.
  assert.equal(getActivePlaygroundSid(), null);
  assert.equal(embedded.getActiveEmbeddedWindow()?.label, "popout-split-ccc");

  // Chrome X must tear down the embed host, same as parkPlaygroundHost.
  dismissPlayground();
  assert.equal(embedded.getActiveEmbeddedWindow(), null);
  assert.equal(getActivePlaygroundSid(), null);
});
