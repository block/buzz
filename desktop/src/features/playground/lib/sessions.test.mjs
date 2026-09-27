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

test("preferSidePanel hosts in RHS and clears on park / reset", async () => {
  const {
    addPlaygroundSession,
    configurePlaygroundScope,
    getPlaygroundOverlayHost,
    isPlaygroundSidePanelHost,
    parkPlaygroundHost,
    resetPlaygroundState,
    showPlaygroundSession,
  } = await import("./sessions.ts");

  configurePlaygroundScope("pub", "wss://relay.example.com");
  assert.equal(isPlaygroundSidePanelHost(), false);
  assert.equal(getPlaygroundOverlayHost(), "window");

  addPlaygroundSession(card, { preferSidePanel: true });
  assert.equal(isPlaygroundSidePanelHost(), true);
  assert.equal(getPlaygroundOverlayHost(), "side-panel");

  showPlaygroundSession("demo-1"); // default window host
  assert.equal(isPlaygroundSidePanelHost(), false);
  assert.equal(getPlaygroundOverlayHost(), "window");

  showPlaygroundSession("demo-1", { preferSidePanel: true });
  parkPlaygroundHost();
  assert.equal(isPlaygroundSidePanelHost(), false);
  assert.equal(getPlaygroundOverlayHost(), "window");

  showPlaygroundSession("demo-1", { preferSidePanel: true });
  resetPlaygroundState();
  assert.equal(isPlaygroundSidePanelHost(), false);
});


test("tab switch and addPlaygroundTab preserve side-panel host (no fullscreen promote)", async () => {
  const {
    addPlaygroundSession,
    addPlaygroundTab,
    configurePlaygroundScope,
    getPlaygroundOverlayHost,
    isPlaygroundSidePanelHost,
    switchPlaygroundTab,
  } = await import("./sessions.ts");

  configurePlaygroundScope("pub", "wss://relay.example.com");
  addPlaygroundSession(card, { preferSidePanel: true });
  assert.equal(isPlaygroundSidePanelHost(), true);

  const browserId = "demo-1";
  const tab = addPlaygroundTab({
    browserId,
    url: "https://sibling.example.com",
    name: "Sibling",
  });
  assert.ok(tab);
  assert.equal(isPlaygroundSidePanelHost(), true);
  assert.equal(getPlaygroundOverlayHost(), "side-panel");

  switchPlaygroundTab("demo-1");
  assert.equal(isPlaygroundSidePanelHost(), true);
  assert.equal(getPlaygroundOverlayHost(), "side-panel");

  switchPlaygroundTab(tab.sid);
  assert.equal(isPlaygroundSidePanelHost(), true);
  assert.equal(getPlaygroundOverlayHost(), "side-panel");

  // Explicit false still forces window host.
  switchPlaygroundTab("demo-1", { preferSidePanel: false });
  assert.equal(isPlaygroundSidePanelHost(), false);
  assert.equal(getPlaygroundOverlayHost(), "window");
});

test("closePlaygroundTab preserves side-panel host (no inset-0 fullscreen promote)", async () => {
  const {
    addPlaygroundSession,
    addPlaygroundTab,
    closePlaygroundTab,
    configurePlaygroundScope,
    getActivePlaygroundSid,
    getPlaygroundOverlayHost,
    isPlaygroundSidePanelHost,
  } = await import("./sessions.ts");

  configurePlaygroundScope("pub", "wss://relay.example.com");
  addPlaygroundSession(card, { preferSidePanel: true });
  assert.equal(isPlaygroundSidePanelHost(), true);

  const tab = addPlaygroundTab({
    browserId: "demo-1",
    url: "https://sibling.example.com",
    name: "Sibling",
  });
  assert.ok(tab);
  assert.equal(getActivePlaygroundSid(), tab.sid);
  assert.equal(getPlaygroundOverlayHost(), "side-panel");

  // Closing the *active* secondary tab used to dispose before reassigning
  // overlaySid, which reset overlayHost to "window" and mounted the
  // absolute inset-0 PlaygroundOverlay over the whole channel canvas.
  closePlaygroundTab(tab.sid);
  assert.equal(getActivePlaygroundSid(), "demo-1");
  assert.equal(isPlaygroundSidePanelHost(), true);
  assert.equal(getPlaygroundOverlayHost(), "side-panel");
});

test("add creates a one-tab browser group; tab add/switch/close works", async () => {
  const {
    addPlaygroundSession,
    addPlaygroundTab,
    closePlaygroundTab,
    configurePlaygroundScope,
    getActivePlaygroundSid,
    getBrowserForSid,
    listPlaygroundBrowsers,
    listPlaygroundSessions,
    switchPlaygroundTab,
  } = await import("./sessions.ts");

  configurePlaygroundScope("pub", "wss://relay.example.com");
  addPlaygroundSession(card);
  const browser = getBrowserForSid("demo-1");
  assert.ok(browser);
  assert.equal(browser.browserId, "demo-1");
  assert.deepEqual(browser.tabSids, ["demo-1"]);
  assert.equal(listPlaygroundBrowsers().length, 1);

  const tab = addPlaygroundTab({
    browserId: browser.browserId,
    url: "https://other.example.com",
    name: "Other",
  });
  assert.ok(tab);
  assert.equal(getActivePlaygroundSid(), tab.sid);
  assert.equal(getBrowserForSid(tab.sid)?.tabSids.length, 2);
  assert.equal(listPlaygroundSessions().length, 2);

  switchPlaygroundTab("demo-1");
  assert.equal(getActivePlaygroundSid(), "demo-1");
  assert.equal(getBrowserForSid("demo-1")?.activeTabSid, "demo-1");

  closePlaygroundTab(tab.sid);
  assert.equal(listPlaygroundSessions().length, 1);
  assert.equal(getBrowserForSid("demo-1")?.tabSids.length, 1);

  // Main/primary tab cannot be dismissed via closePlaygroundTab — use Browsers Remove.
  closePlaygroundTab("demo-1");
  assert.equal(listPlaygroundSessions().length, 1);
  assert.equal(listPlaygroundBrowsers().length, 1);
});

test("disposePlaygroundBrowser removes conversation pins for every tab sid", async () => {
  const {
    addPlaygroundSession,
    configurePlaygroundScope,
    disposePlaygroundBrowser,
    getBrowserForSid,
  } = await import("./sessions.ts");
  const {
    listConversationPlaygroundPins,
    pinPlaygroundToConversation,
  } = await import("./conversationPins.ts");

  configurePlaygroundScope("pub", "wss://relay.example.com");
  addPlaygroundSession(card);
  pinPlaygroundToConversation("channel:chan-a", card);
  pinPlaygroundToConversation("thread:root-1", card, "chan-a");
  assert.equal(listConversationPlaygroundPins("channel:chan-a").length, 1);
  assert.equal(listConversationPlaygroundPins("thread:root-1").length, 1);

  const browser = getBrowserForSid("demo-1");
  assert.ok(browser);
  disposePlaygroundBrowser(browser.browserId);
  assert.equal(listConversationPlaygroundPins("channel:chan-a").length, 0);
  assert.equal(listConversationPlaygroundPins("thread:root-1").length, 0);
});

test("pins for a live session survive configurePlaygroundScope restart hydrate", async () => {
  const {
    addPlaygroundSession,
    configurePlaygroundScope,
    listPlaygroundSessions,
    resetPlaygroundState,
  } = await import("./sessions.ts");
  const { listConversationPlaygroundPins, pinPlaygroundToConversation } =
    await import("./conversationPins.ts");

  configurePlaygroundScope("pub", "wss://relay.example.com");
  addPlaygroundSession(card);
  pinPlaygroundToConversation("channel:chan-a", card);
  assert.equal(listConversationPlaygroundPins("channel:chan-a").length, 1);

  resetPlaygroundState();
  assert.equal(listPlaygroundSessions().length, 0);
  assert.equal(listConversationPlaygroundPins("channel:chan-a").length, 0);

  configurePlaygroundScope("pub", "wss://relay.example.com");
  assert.equal(listPlaygroundSessions().length, 1);
  assert.equal(listConversationPlaygroundPins("channel:chan-a").length, 1);
});
