import assert from "node:assert/strict";
import test from "node:test";

import { installLocalStorage } from "../../playground/lib/testStorage.mjs";

import { popoutErrorMessage, popoutLabel } from "./popoutWindow.ts";

function installTauriInvoke() {
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
  return invokes;
}

function uninstallTauriInvoke() {
  delete globalThis.__TAURI_INTERNALS__;
  delete globalThis.isTauri;
  if (globalThis.window) {
    delete globalThis.window.__TAURI_INTERNALS__;
  }
}

test("split labels stay unique when sid+channel prefixes collide at 48 chars", () => {
  const sid = `pg-${"s".repeat(40)}`;
  const channelId = `chan-${"c".repeat(40)}`;
  const prefix = `${sid}-${channelId}-`;
  assert.ok(prefix.length > 48);

  const seedA = `${sid}-${channelId}-thread-aaa`;
  const seedB = `${sid}-${channelId}-thread-bbb`;
  const labelA = popoutLabel("split", seedA);
  const labelB = popoutLabel("split", seedB);

  assert.notEqual(labelA, labelB);
  assert.equal(popoutLabel("split", seedA), labelA);
  assert.match(labelA, /^popout-split-[0-9a-f]{12,16}$/);
  assert.match(labelB, /^popout-split-[0-9a-f]{12,16}$/);
  assert.ok(labelA.length < 48);
  assert.ok(labelB.length < 48);
});

test("same playground+thread split keeps the same label", () => {
  const seed = "sid-channel-thread-1";
  assert.equal(popoutLabel("split", seed), popoutLabel("split", seed));
});

test("popoutErrorMessage prefers Error and string throws over the fallback", () => {
  assert.equal(
    popoutErrorMessage(
      new Error("label already exists"),
      "Could not open split.",
    ),
    "label already exists",
  );
  assert.equal(
    popoutErrorMessage("label already exists", "Could not open split."),
    "label already exists",
  );
  assert.equal(
    popoutErrorMessage({ reason: "nope" }, "Could not open split."),
    "Could not open split.",
  );
});

test("start-fullscreen is passed into the OS create payload", async () => {
  installLocalStorage();
  const { resetPopoutSettingsForTests, setStartFullscreen } = await import(
    "./popoutSettings.ts"
  );
  const { popoutCreateInvokeArgs } = await import("./popoutWindow.ts");
  resetPopoutSettingsForTests();
  setStartFullscreen(false);
  assert.deepEqual(
    popoutCreateInvokeArgs({ label: "popout-thread-aaa", title: "Thread" }),
    { label: "popout-thread-aaa", title: "Thread", fullscreen: false },
  );
  setStartFullscreen(true);
  assert.deepEqual(
    popoutCreateInvokeArgs({ label: "popout-thread-aaa", title: "Thread" }),
    { label: "popout-thread-aaa", title: "Thread", fullscreen: true },
  );
  resetPopoutSettingsForTests();
});

test("embed path does not call OS window create for playground", async () => {
  installLocalStorage();
  const settings = await import("./popoutSettings.ts");
  const embedded = await import("./embeddedWindows.ts");
  const { openPopoutWindow } = await import("./popoutWindow.ts");
  settings.resetPopoutSettingsForTests();
  embedded.resetEmbeddedWindowsForTests();
  settings.setShowWindowsSection(true);
  settings.setEmbedInMain(true);

  const invokes = [];
  const internals = {
    invoke(cmd, args) {
      invokes.push({ cmd, args });
      return Promise.resolve();
    },
  };
  globalThis.__TAURI_INTERNALS__ = internals;

  await openPopoutWindow({
    kind: "playground",
    title: "Demo",
    seed: "demo-1",
    playground: {
      hula: "playground",
      v: 1,
      name: "Demo",
      url: "https://app.example.com",
      sid: "demo-1",
    },
  });

  assert.equal(invokes.length, 0);
  assert.equal(embedded.listEmbeddedWindows().length, 1);
  assert.equal(embedded.getActiveEmbeddedWindow()?.payload.kind, "playground");
  assert.equal(
    embedded.getActiveEmbeddedWindow()?.payload.playground?.sid,
    "demo-1",
  );

  delete globalThis.__TAURI_INTERNALS__;
  settings.resetPopoutSettingsForTests();
  embedded.resetEmbeddedWindowsForTests();
});

test("channel/thread opens OS window even when embed-in-main is on", async () => {
  installLocalStorage();
  const settings = await import("./popoutSettings.ts");
  const embedded = await import("./embeddedWindows.ts");
  const { openPopoutWindow } = await import("./popoutWindow.ts");
  settings.resetPopoutSettingsForTests();
  embedded.resetEmbeddedWindowsForTests();
  settings.setShowWindowsSection(true);
  settings.setEmbedInMain(true);

  const invokes = installTauriInvoke();

  await openPopoutWindow({
    kind: "thread",
    title: "Design review",
    seed: "chan-thread-1",
    channelId: "chan",
    threadId: "thread-1",
  });

  assert.equal(embedded.listEmbeddedWindows().length, 0);
  assert.equal(invokes.length, 1);
  assert.equal(invokes[0].cmd, "open_popout_window");
  assert.match(invokes[0].args.label, /^popout-thread-/);

  uninstallTauriInvoke();
  settings.resetPopoutSettingsForTests();
  embedded.resetEmbeddedWindowsForTests();
});

test("link Detach opens OS window even when embed-in-main is on", async () => {
  installLocalStorage();
  const settings = await import("./popoutSettings.ts");
  const embedded = await import("./embeddedWindows.ts");
  const { openPopoutWindow } = await import("./popoutWindow.ts");
  settings.resetPopoutSettingsForTests();
  embedded.resetEmbeddedWindowsForTests();
  settings.setShowWindowsSection(true);
  settings.setEmbedInMain(true);

  const invokes = installTauriInvoke();

  await openPopoutWindow({
    kind: "link",
    title: "Port Hole",
    seed: "pin-port-hole",
    forceOsWindow: true,
    link: {
      url: "https://hula-port-hole.hulapreview.com",
      pinId: "pin-port-hole",
      viewportMode: "desktop",
      keepAlive: false,
    },
  });

  assert.equal(embedded.listEmbeddedWindows().length, 0);
  assert.equal(invokes.length, 1);
  assert.equal(invokes[0].cmd, "open_popout_window");
  assert.match(invokes[0].args.label, /^popout-link-/);

  uninstallTauriInvoke();
  settings.resetPopoutSettingsForTests();
  embedded.resetEmbeddedWindowsForTests();
});

test("OS split parks the main overlay playground", async () => {
  installLocalStorage();
  const settings = await import("./popoutSettings.ts");
  const embedded = await import("./embeddedWindows.ts");
  const sessions = await import("../../playground/lib/sessions.ts");
  const { openPopoutWindow } = await import("./popoutWindow.ts");
  settings.resetPopoutSettingsForTests();
  embedded.resetEmbeddedWindowsForTests();
  sessions.resetPlaygroundState();

  sessions.configurePlaygroundScope("pub", "wss://relay.example.com");
  sessions.addPlaygroundSession({
    hula: "playground",
    v: 1,
    name: "Demo",
    url: "https://app.example.com",
    sid: "demo-1",
  });
  assert.equal(sessions.getActivePlaygroundSid(), "demo-1");

  await openPopoutWindow({
    kind: "split",
    title: "Split Demo",
    seed: "demo-1-chan-thread",
    channelId: "chan",
    threadId: "thread-1",
    playground: {
      hula: "playground",
      v: 1,
      name: "Demo",
      url: "https://app.example.com",
      sid: "demo-1",
    },
  });

  assert.equal(sessions.getActivePlaygroundSid(), null);
  assert.equal(embedded.listEmbeddedWindows().length, 0);

  settings.resetPopoutSettingsForTests();
  embedded.resetEmbeddedWindowsForTests();
  sessions.resetPlaygroundState();
});

test("isPopoutSplitLayout is true only for kind split", async () => {
  const { isPopoutSplitLayout, isPopoutThreadOnlyLayout } = await import(
    "./popoutWindow.ts"
  );
  assert.equal(isPopoutSplitLayout(null), false);
  assert.equal(
    isPopoutSplitLayout({ kind: "thread", threadId: "t", channelId: "c" }),
    false,
  );
  assert.equal(
    isPopoutSplitLayout({
      kind: "playground",
      playground: {
        hula: "playground",
        v: 1,
        name: "Demo",
        url: "https://app.example.com",
        sid: "demo-1",
      },
    }),
    false,
  );
  assert.equal(
    isPopoutSplitLayout({ kind: "split", threadId: "t", channelId: "c" }),
    true,
  );
  // Thread-only still covers both thread and split; split is the narrower gate.
  assert.equal(
    isPopoutThreadOnlyLayout({ kind: "thread", threadId: "t" }),
    true,
  );
  assert.equal(
    isPopoutThreadOnlyLayout({ kind: "split", threadId: "t" }),
    true,
  );
  assert.equal(isPopoutThreadOnlyLayout({ kind: "split" }), false);
});

test("isPopoutForcedSinglePanelView lifts plain thread pop-outs for Activity", async () => {
  const { isPopoutForcedSinglePanelView } = await import("./popoutWindow.ts");

  assert.equal(
    isPopoutForcedSinglePanelView({
      hasNonThreadAuxiliary: false,
      isPopoutPlaygroundSplit: false,
      isPopoutThreadOnly: true,
    }),
    true,
  );
  assert.equal(
    isPopoutForcedSinglePanelView({
      hasNonThreadAuxiliary: true,
      isPopoutPlaygroundSplit: false,
      isPopoutThreadOnly: true,
    }),
    false,
  );
  // Playground split keeps single-panel content; Activity replaces the thread.
  assert.equal(
    isPopoutForcedSinglePanelView({
      hasNonThreadAuxiliary: true,
      isPopoutPlaygroundSplit: true,
      isPopoutThreadOnly: true,
    }),
    true,
  );
  assert.equal(
    isPopoutForcedSinglePanelView({
      hasNonThreadAuxiliary: true,
      isPopoutPlaygroundSplit: false,
      isPopoutThreadOnly: false,
    }),
    false,
  );
});

test("shouldSeedPopoutChannelRoute only seeds once for channel/thread companions", async () => {
  const { shouldSeedPopoutChannelRoute } = await import("./popoutWindow.ts");

  assert.equal(
    shouldSeedPopoutChannelRoute({
      alreadySeeded: false,
      payload: { kind: "thread", channelId: "c", threadId: "t" },
    }),
    true,
  );
  assert.equal(
    shouldSeedPopoutChannelRoute({
      alreadySeeded: true,
      payload: { kind: "thread", channelId: "c", threadId: "t" },
    }),
    false,
  );
  assert.equal(
    shouldSeedPopoutChannelRoute({
      alreadySeeded: false,
      payload: { kind: "link", link: { url: "https://x", pinId: "p" } },
    }),
    false,
  );
  assert.equal(
    shouldSeedPopoutChannelRoute({
      alreadySeeded: false,
      payload: { kind: "thread" },
    }),
    false,
  );
});

test("popoutPayloadFromInput snapshots playground pins for thread/split", async () => {
  const pins = await import("@/features/playground/lib/conversationPins.ts");
  pins.resetConversationPlaygroundPins();
  pins.pinPlaygroundToConversation("thread:thread-1", {
    hula: "playground",
    v: 1,
    name: "Demo",
    url: "https://app.example.com",
    sid: "demo-1",
    pin: "4455",
  });
  const { popoutPayloadFromInput, popoutPlaygroundPinsScopeKey } = await import(
    "./popoutWindow.ts"
  );

  assert.equal(
    popoutPlaygroundPinsScopeKey("chan", "thread-1"),
    "thread:thread-1",
  );
  assert.equal(popoutPlaygroundPinsScopeKey("chan", null), "channel:chan");

  const threadPayload = popoutPayloadFromInput({
    kind: "thread",
    title: "Thread",
    channelId: "chan",
    threadId: "thread-1",
  });
  assert.equal(threadPayload.playgroundPins?.length, 1);
  assert.equal(threadPayload.playgroundPins?.[0]?.sid, "demo-1");

  const splitPayload = popoutPayloadFromInput({
    kind: "split",
    title: "Split",
    channelId: "chan",
    threadId: "thread-1",
    playground: {
      hula: "playground",
      v: 1,
      name: "Demo",
      url: "https://app.example.com",
      sid: "demo-1",
    },
  });
  assert.equal(splitPayload.playgroundPins?.length, 1);

  const playgroundOnly = popoutPayloadFromInput({
    kind: "playground",
    title: "Demo",
    channelId: "chan",
    threadId: "thread-1",
    playground: {
      hula: "playground",
      v: 1,
      name: "Demo",
      url: "https://app.example.com",
      sid: "demo-1",
    },
  });
  assert.equal(playgroundOnly.playgroundPins, undefined);

  pins.resetConversationPlaygroundPins();
});

test("openPopoutWindow writes playgroundPins into localStorage payload", async () => {
  installLocalStorage();
  const pins = await import("@/features/playground/lib/conversationPins.ts");
  pins.resetConversationPlaygroundPins();
  pins.pinPlaygroundToConversation("thread:thread-1", {
    hula: "playground",
    v: 1,
    name: "Demo",
    url: "https://app.example.com",
    sid: "demo-1",
  });
  const settings = await import("./popoutSettings.ts");
  const embedded = await import("./embeddedWindows.ts");
  const { openPopoutWindow, popoutLabel, readPopoutPayload } = await import(
    "./popoutWindow.ts"
  );
  settings.resetPopoutSettingsForTests();
  embedded.resetEmbeddedWindowsForTests();
  settings.setEmbedInMain(false);

  const invokes = installTauriInvoke();
  await openPopoutWindow({
    kind: "thread",
    title: "Design review",
    seed: "chan-thread-pins",
    channelId: "chan",
    threadId: "thread-1",
  });
  const label = popoutLabel("thread", "chan-thread-pins");
  const payload = readPopoutPayload(label);
  assert.equal(payload?.playgroundPins?.length, 1);
  assert.equal(payload?.playgroundPins?.[0]?.name, "Demo");
  assert.equal(invokes.length, 1);

  uninstallTauriInvoke();
  settings.resetPopoutSettingsForTests();
  embedded.resetEmbeddedWindowsForTests();
  pins.resetConversationPlaygroundPins();
});

test("popoutPayloadFromInput snapshots channel-scoped pins for channel companions", async () => {
  const pins = await import("@/features/playground/lib/conversationPins.ts");
  pins.resetConversationPlaygroundPins();
  pins.pinPlaygroundToConversation("channel:chan", {
    hula: "playground",
    v: 1,
    name: "Channel Demo",
    url: "https://channel.example.com",
    sid: "chan-demo-1",
  });
  // Thread pin must not leak into a channel-only companion snapshot.
  pins.pinPlaygroundToConversation("thread:thread-1", {
    hula: "playground",
    v: 1,
    name: "Thread Demo",
    url: "https://thread.example.com",
    sid: "thread-demo-1",
  });
  const { popoutPayloadFromInput } = await import("./popoutWindow.ts");

  const channelPayload = popoutPayloadFromInput({
    kind: "thread",
    title: "Channel",
    channelId: "chan",
    // No threadId — ConversationPopoutMenu channel detach path.
  });
  assert.equal(channelPayload.playgroundPins?.length, 1);
  assert.equal(channelPayload.playgroundPins?.[0]?.sid, "chan-demo-1");
  assert.equal(channelPayload.playgroundPins?.[0]?.name, "Channel Demo");

  pins.resetConversationPlaygroundPins();
});

test("openPopoutWindow writes channel playgroundPins without threadId", async () => {
  installLocalStorage();
  const pins = await import("@/features/playground/lib/conversationPins.ts");
  pins.resetConversationPlaygroundPins();
  pins.pinPlaygroundToConversation("channel:chan", {
    hula: "playground",
    v: 1,
    name: "Home App",
    url: "https://home.example.com",
    sid: "home-1",
  });
  const settings = await import("./popoutSettings.ts");
  const embedded = await import("./embeddedWindows.ts");
  const { openPopoutWindow, popoutLabel, readPopoutPayload } = await import(
    "./popoutWindow.ts"
  );
  settings.resetPopoutSettingsForTests();
  embedded.resetEmbeddedWindowsForTests();
  settings.setEmbedInMain(false);

  const invokes = installTauriInvoke();
  await openPopoutWindow({
    kind: "thread",
    title: "Channel",
    seed: "chan",
    channelId: "chan",
  });
  const label = popoutLabel("thread", "chan");
  const payload = readPopoutPayload(label);
  assert.equal(payload?.playgroundPins?.length, 1);
  assert.equal(payload?.playgroundPins?.[0]?.sid, "home-1");
  assert.equal(invokes.length, 1);

  uninstallTauriInvoke();
  settings.resetPopoutSettingsForTests();
  embedded.resetEmbeddedWindowsForTests();
  pins.resetConversationPlaygroundPins();
});
