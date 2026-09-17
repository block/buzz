import assert from "node:assert/strict";
import { afterEach, before, test } from "node:test";

before(() => {
  globalThis.__TAURI_INTERNALS__ = {
    invoke: async () => undefined,
  };
});

afterEach(async () => {
  const { resetLinkSidePanelStore } = await import("./linkSidePanelStore.ts");
  resetLinkSidePanelStore();
});

test("openLinkSidePanel accepts http(s) and rejects other schemes", async () => {
  const {
    getLinkSidePanel,
    isLinkSidePanelUrl,
    openLinkSidePanel,
  } = await import("./linkSidePanelStore.ts");

  assert.equal(isLinkSidePanelUrl("https://example.com/path"), true);
  assert.equal(isLinkSidePanelUrl("http://localhost:3000"), true);
  assert.equal(isLinkSidePanelUrl("hulabuzz://message?id=1"), false);
  assert.equal(isLinkSidePanelUrl("not a url"), false);

  assert.equal(openLinkSidePanel("https://www.example.com/docs"), true);
  const panel = getLinkSidePanel();
  assert.ok(panel);
  assert.equal(panel.url, "https://www.example.com/docs");
  assert.equal(panel.title, "example.com");
  assert.equal(panel.expanded, false);

  assert.equal(openLinkSidePanel("hulabuzz://x"), false);
});

test("expand toggle and close clear the panel", async () => {
  const {
    closeLinkSidePanel,
    getLinkSidePanel,
    openLinkSidePanel,
    toggleLinkSidePanelExpanded,
  } = await import("./linkSidePanelStore.ts");

  openLinkSidePanel("https://huladesk.com");
  toggleLinkSidePanelExpanded();
  assert.equal(getLinkSidePanel()?.expanded, true);
  toggleLinkSidePanelExpanded();
  assert.equal(getLinkSidePanel()?.expanded, false);
  closeLinkSidePanel();
  assert.equal(getLinkSidePanel(), null);
});

test("LINK_SIDE_PANEL_PIN_ID is stable for native teardown", async () => {
  const { LINK_SIDE_PANEL_PIN_ID } = await import("./linkSidePanelStore.ts");
  assert.equal(LINK_SIDE_PANEL_PIN_ID, "hula-link-side-panel");
});

test("openLinkSidePanel keepAlive uses a dedicated pinId and survives close via hide", async () => {
  const {
    closeLinkSidePanel,
    destroyLinkSidePanelIfPin,
    getLinkSidePanel,
    openLinkSidePanel,
  } = await import("./linkSidePanelStore.ts");

  assert.equal(
    openLinkSidePanel("https://app.example.com", {
      title: "Demo",
      pinId: "playground-pin-demo-1",
      keepAlive: true,
    }),
    true,
  );
  const panel = getLinkSidePanel();
  assert.equal(panel?.title, "Demo");
  assert.equal(panel?.pinId, "playground-pin-demo-1");
  assert.equal(panel?.keepAlive, true);

  closeLinkSidePanel();
  assert.equal(getLinkSidePanel(), null);

  // Re-open then destroy via unpin helper.
  openLinkSidePanel("https://app.example.com", {
    pinId: "playground-pin-demo-1",
    keepAlive: true,
  });
  destroyLinkSidePanelIfPin("playground-pin-demo-1");
  assert.equal(getLinkSidePanel(), null);
});
