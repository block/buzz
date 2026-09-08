/**
 * Widget-boundary coverage for the shared <PubKey> identity gate.
 *
 * Codec vectors and the exact compact/neutral strings live in
 * ../lib/pubkey.test.mjs. This suite pins what static rendering shows: the
 * rendered text per variant, and that an unencodable identity — including
 * degenerate-length hex and short-payload npubs whose npubEncode outputs
 * carry valid checksums — renders the neutral label with no copy affordance,
 * never a fake npub. The clipboard write behind the copy affordance and the
 * popover the widget opens are real-bridge interactions owned by the E2E
 * regressions: the full variant's copy is pinned by the new-DM recipient
 * verification flow (tests/e2e/pubkey-display-screenshots.spec.ts) and the
 * compact variant's by the agent-access owner hint
 * (tests/e2e/agent-access-warning.spec.ts); both drive CopyRow through the
 * mock bridge into the actual browser clipboard.
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
  // would happily turn into a checksum-valid fake npub; npub1m6kmamcvty5gd
  // and npub106246s decode fine but are checksum-valid short-payload npubs
  // (8-char and empty identity payloads). All four would masquerade as
  // displayable identities — the gate refuses every one.
  for (const pubkey of [
    "zz",
    "deadbeef",
    "npub1m6kmamcvty5gd",
    "npub106246s",
  ]) {
    for (const variant of [undefined, "full"]) {
      const view = await renderPubKey({ pubkey, variant });
      const label = `${pubkey} ${variant ?? "compact"}`;
      assert.equal(view.getByText("Unavailable").tagName, "SPAN", label);
      assert.equal(view.queryByRole("button"), null, label);
      assert.equal(view.container.textContent?.includes("npub1"), false, label);
    }
  }
});
