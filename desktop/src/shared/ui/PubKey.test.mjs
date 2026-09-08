/**
 * Widget-boundary coverage for the shared <PubKey> identity gate.
 *
 * Codec vectors and the exact compact/neutral strings live in
 * ../lib/pubkey.test.mjs; real clipboard contents and the npub-only verify
 * popover are exercised end-to-end (tests/e2e/profile.spec.ts copies the
 * canonical npub; pubkey-display-screenshots.spec.ts mounts this widget).
 * This slim local suite pins only the widget wiring those harnesses cannot:
 * the rendered text per variant, and that an unencodable identity — including
 * a degenerate-length hex whose npubEncode output carries a valid checksum —
 * renders the neutral label with no copy affordance, never a fake npub.
 */
import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    getComputedStyle: dom.window.getComputedStyle.bind(dom.window),
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    Node: dom.window.Node,
    ResizeObserver: class {
      disconnect() {}
      observe() {}
      unobserve() {}
    },
    window: dom.window,
  });
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
});

after(() => dom.window.close());

const HEX = "ea9b4d7a7a78a3e3729e5568b14d764d4962be0e1f20f749bcf8d9dbbf9a9328";
const NPUB = "npub1a2d567n60z37xu57245tzntkf4yk90swrus0wjdulrvah0u6jv5qusyp60";
const COMPACT_NPUB = "npub1a2d…yp60";

async function renderPubKey(props) {
  const React = await import("react");
  const { render, within } = await import("@testing-library/react");
  const { PubKey } = await import("./PubKey.tsx");
  const view = render(React.createElement(PubKey, props));
  // render()'s bound queries search the whole body; scope to this render so
  // earlier mounts (cleaned up only per test) stay invisible.
  return { ...within(view.container), container: view.container };
}

test("compact PubKey renders the truncated npub, never the hex", async () => {
  const trigger = await renderPubKey({ pubkey: HEX });
  assert.equal(
    trigger.getByRole("button", { name: "Show full public key" }).textContent,
    COMPACT_NPUB,
  );
  assert.equal(trigger.queryByText(HEX), null);

  // A parent row that owns the interaction gets the same text, not a button.
  const text = await renderPubKey({ interactive: false, pubkey: HEX });
  assert.equal(text.getByText(COMPACT_NPUB).tagName, "SPAN");
  assert.equal(text.queryByRole("button"), null);
  assert.equal(text.queryByText(HEX), null);

  // An all-uppercase Bech32 npub is a valid identity (parsePubkeyInput
  // accepts it); the gate must render its canonical compact form, not the
  // neutral label.
  const upper = await renderPubKey({ pubkey: NPUB.toUpperCase() });
  assert.equal(
    upper.getByRole("button", { name: "Show full public key" }).textContent,
    COMPACT_NPUB,
  );
  assert.equal(upper.queryByText("Unavailable"), null);
});

test("full PubKey renders the complete npub with a copy affordance", async () => {
  const view = await renderPubKey({ pubkey: HEX, variant: "full" });
  assert.equal(view.getByText(NPUB).textContent, NPUB);
  assert.equal(
    view.getByRole("button", { name: "Copy public key" }).tagName,
    "BUTTON",
  );
  assert.equal(view.queryByText(HEX), null);
});

test("unencodable keys render Unavailable with no copy affordance", async () => {
  // "zz" cannot decode; "deadbeef" is a degenerate-length hex that npubEncode
  // would happily turn into a checksum-valid fake npub — refuse both.
  for (const pubkey of ["zz", "deadbeef"]) {
    for (const variant of [undefined, "full"]) {
      const view = await renderPubKey({ pubkey, variant });
      const label = `${pubkey} ${variant ?? "compact"}`;
      assert.equal(view.getByText("Unavailable").tagName, "SPAN", label);
      assert.equal(view.queryByRole("button"), null, label);
      assert.equal(view.container.textContent?.includes("npub1"), false, label);
    }
  }
});
