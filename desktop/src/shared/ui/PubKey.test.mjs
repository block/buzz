import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

before(() => {
  Object.assign(globalThis, {
    CustomEvent: dom.window.CustomEvent,
    document: dom.window.document,
    Element: dom.window.Element,
    Event: dom.window.Event,
    getComputedStyle: dom.window.getComputedStyle.bind(dom.window),
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    Node: dom.window.Node,
    NodeFilter: dom.window.NodeFilter,
    ResizeObserver: class {
      disconnect() {}
      observe() {}
      unobserve() {}
    },
    window: dom.window,
  });
  // Copy remaining DOM-level globals that Radix Popover focus machinery
  // references without a `window.` prefix (NodeFilter, HTMLInputElement,
  // …). Bulk-copy follows the HarnessCatalogDialog.acpForcedGate test pattern
  // so new Radix internals cannot reintroduce per-global whack-a-mole.
  for (const key of Object.getOwnPropertyNames(dom.window)) {
    if (
      !(key in globalThis) &&
      (key.startsWith("HTML") ||
        key.startsWith("SVG") ||
        key.startsWith("CSS") ||
        [
          "Node",
          "NodeFilter",
          "NodeList",
          "NamedNodeMap",
          "Event",
          "CustomEvent",
          "MouseEvent",
          "KeyboardEvent",
          "FocusEvent",
          "InputEvent",
          "PointerEvent",
          "EventTarget",
          "Text",
          "DocumentFragment",
          "Range",
          "Selection",
        ].includes(key))
    ) {
      const val = dom.window[key];
      if (val !== undefined) globalThis[key] = val;
    }
  }
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
});

after(() => dom.window.close());

const HEX = "ea9b4d7a7a78a3e3729e5568b14d764d4962be0e1f20f749bcf8d9dbbf9a9328";
const NPUB = "npub1a2d567n60z37xu57245tzntkf4yk90swrus0wjdulrvah0u6jv5qusyp60";
const COMPACT_NPUB = "npub1a2d…yp60";

test("compact PubKey renders the truncated npub, never the hex", async () => {
  const React = await import("react");
  const { render } = await import("@testing-library/react");
  const { PubKey } = await import("./PubKey.tsx");

  const view = render(React.createElement(PubKey, { pubkey: HEX }));

  const trigger = view.getByRole("button", { name: "Show full public key" });
  assert.equal(trigger.textContent, COMPACT_NPUB);
  assert.equal(view.queryByText(HEX), null);
});

test("non-interactive compact PubKey renders the truncated npub as text", async () => {
  const React = await import("react");
  const { render } = await import("@testing-library/react");
  const { PubKey } = await import("./PubKey.tsx");

  const view = render(
    React.createElement(PubKey, { interactive: false, pubkey: HEX }),
  );

  assert.equal(view.getByText(COMPACT_NPUB).tagName, "SPAN");
  assert.equal(view.queryByRole("button"), null);
  assert.equal(view.queryByText(HEX), null);
});

test("full PubKey renders the complete npub with an npub-only copy popover", async () => {
  const React = await import("react");
  const { fireEvent, render, within } = await import("@testing-library/react");
  const { PubKey } = await import("./PubKey.tsx");

  const view = render(
    React.createElement(PubKey, { pubkey: HEX, variant: "full" }),
  );

  assert.equal(within(view.container).getByText(NPUB).textContent, NPUB);
  assert.equal(view.queryByText(HEX), null);

  fireEvent.click(
    within(view.container).getByRole("button", { name: "Copy public key" }),
  );
  // The copy popover offers the full canonical npub only — no hex row.
  assert.equal(view.getAllByText(NPUB).length, 2);
  assert.equal(
    view.getByRole("button", { name: "Copy npub" }).tagName,
    "BUTTON",
  );
  assert.equal(view.queryByRole("button", { name: "Copy hex" }), null);
  assert.equal(view.queryByText("hex"), null);
});

test("invalid keys render Unavailable with no copy affordance", async () => {
  const React = await import("react");
  const { render, within } = await import("@testing-library/react");
  const { PubKey } = await import("./PubKey.tsx");

  const compact = render(React.createElement(PubKey, { pubkey: "zz" }));
  assert.equal(
    within(compact.container).getByText("Unavailable").tagName,
    "SPAN",
  );
  assert.equal(
    within(compact.container).queryByRole("button", {
      name: "Show full public key",
    }),
    null,
  );

  const full = render(
    React.createElement(PubKey, { pubkey: "zz", variant: "full" }),
  );
  assert.equal(within(full.container).getByText("Unavailable").tagName, "SPAN");
  // No copy action for an identity that cannot be encoded.
  assert.equal(
    within(full.container).queryByRole("button", { name: "Copy public key" }),
    null,
  );
});

test("degenerate-length hex keys render Unavailable, never a fake npub", async () => {
  const React = await import("react");
  const { render, within } = await import("@testing-library/react");
  const { PubKey } = await import("./PubKey.tsx");

  // `npubEncode` happily encodes an 8-char hex into a valid-checksum npub —
  // that is not an identity key, so no surface may show or copy it.
  const short = render(React.createElement(PubKey, { pubkey: "deadbeef" }));
  assert.equal(
    within(short.container).getByText("Unavailable").tagName,
    "SPAN",
  );
  assert.equal(
    within(short.container).queryByRole("button", {
      name: "Show full public key",
    }),
    null,
  );

  const shortFull = render(
    React.createElement(PubKey, { pubkey: "deadbeef", variant: "full" }),
  );
  assert.equal(
    within(shortFull.container).getByText("Unavailable").tagName,
    "SPAN",
  );
  assert.equal(
    within(shortFull.container).queryByRole("button", {
      name: "Copy public key",
    }),
    null,
  );
  assert.equal(shortFull.container.textContent?.includes("npub1"), false);
});
