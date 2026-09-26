import assert from "node:assert/strict";
import { afterEach, before, test } from "node:test";

import { installLocalStorage } from "./testStorage.mjs";

const card = {
  hula: "playground",
  v: 1,
  name: "Demo",
  url: "https://app.example.com",
  sid: "demo-1",
};

before(() => {
  installLocalStorage();
});

afterEach(async () => {
  const { resetPlaygroundState } = await import("./sessions.ts");
  resetPlaygroundState();
  globalThis.localStorage?.clear();
});

test("new-tab request adds sibling tab and preserves side-panel host", async () => {
  const {
    addPlaygroundSession,
    configurePlaygroundScope,
    getBrowserForSid,
    getPlaygroundOverlayHost,
    isPlaygroundSidePanelHost,
    listPlaygroundSessions,
  } = await import("./sessions.ts");
  const { handlePlaygroundNewTabRequest } = await import("./runtime.ts");

  configurePlaygroundScope("pub", "wss://relay.example.com");
  addPlaygroundSession(card, { preferSidePanel: true });
  assert.equal(isPlaygroundSidePanelHost(), true);

  const sid = handlePlaygroundNewTabRequest({
    openerSid: "demo-1",
    openerLabel: "playground-demo-1",
    url: "https://other.example.com/path",
  });
  assert.ok(sid);
  assert.notEqual(sid, "demo-1");
  assert.equal(listPlaygroundSessions().length, 2);
  assert.equal(getBrowserForSid("demo-1")?.tabSids.length, 2);
  assert.equal(getPlaygroundOverlayHost(), "side-panel");
  assert.equal(isPlaygroundSidePanelHost(), true);

  assert.equal(
    handlePlaygroundNewTabRequest({
      openerSid: "demo-1",
      url: "about:blank",
    }),
    null,
  );

  // Dedupe same url+opener within window
  assert.equal(
    handlePlaygroundNewTabRequest({
      openerSid: "demo-1",
      url: "https://other.example.com/path",
    }),
    null,
  );
});

test("new-tab request ignores empty and about:blank (wry may Allow those so createWebView can run)", async () => {
  const { addPlaygroundSession, configurePlaygroundScope, listPlaygroundSessions } =
    await import("./sessions.ts");
  const { handlePlaygroundNewTabRequest } = await import("./runtime.ts");
  configurePlaygroundScope("pub", "wss://relay.example.com");
  addPlaygroundSession({
    hula: "playground",
    v: 1,
    name: "Demo",
    url: "https://app.example.com",
    sid: "demo-blank",
  });
  assert.equal(
    handlePlaygroundNewTabRequest({ openerSid: "demo-blank", url: "" }),
    null,
  );
  assert.equal(
    handlePlaygroundNewTabRequest({ openerSid: "demo-blank", url: "about:blank" }),
    null,
  );
  assert.equal(listPlaygroundSessions().length, 1);
});
