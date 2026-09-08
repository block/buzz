/**
 * Consumer regression for the channel-activity working-agent avatar initials
 * (Astra P2 on PR-D2).
 *
 * `WorkingAgentRows` renders unnamed working agents with the generated
 * role-prefixed key label "Agent npub1…". `getInitials` only recognizes a
 * whole compact/full npub as a key label, so before the `initialsLabel` fix
 * every one of those labels collapsed onto the word initials "AN" — losing
 * the distinct fallback initials avatars otherwise produce. This test mounts
 * the exported rows component (the same one the popover mounts) with a real
 * DOM root so `UserAvatar` runs its production fallback-delay path and the
 * rendered initials are what the app actually shows.
 *
 * A profile display name and an aligned roster name are both checked with
 * their authored initials, and the unnamed pair is checked for distinct
 * key-tail initials rather than "AN".
 */

import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

class NoopObserver {
  disconnect() {}
  observe() {}
  unobserve() {}
}

Object.assign(globalThis, {
  IS_REACT_ACT_ENVIRONMENT: true,
  IntersectionObserver: NoopObserver,
  MutationObserver: dom.window.MutationObserver,
  ResizeObserver: NoopObserver,
  document: dom.window.document,
  localStorage: dom.window.localStorage,
  self: dom.window,
  window: dom.window,
});
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  value: dom.window.navigator,
  writable: true,
});
dom.window.matchMedia = () => ({
  matches: false,
  addEventListener() {},
  removeEventListener() {},
});
globalThis.matchMedia = dom.window.matchMedia;
dom.window.requestAnimationFrame = (callback) => setTimeout(callback, 0);
dom.window.cancelAnimationFrame = (id) => clearTimeout(id);
globalThis.requestAnimationFrame = dom.window.requestAnimationFrame;
globalThis.cancelAnimationFrame = dom.window.cancelAnimationFrame;

// Bulk-copy DOM constructors React and Radix reference without a window
// prefix, so components behave as they do in the app window.
for (const key of Object.getOwnPropertyNames(dom.window)) {
  if (
    !(key in globalThis) &&
    (key.startsWith("HTML") ||
      key.startsWith("SVG") ||
      [
        "Element",
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
        "Text",
        "Comment",
        "DocumentFragment",
        "Range",
        "Selection",
      ].includes(key))
  ) {
    const value = dom.window[key];
    if (value !== undefined) globalThis[key] = value;
  }
}

let React;
let act;
let createRoot;
let WorkingAgentRows;
let normalizePubkey;

before(async () => {
  ({ default: React, act } = await import("react"));
  ({ createRoot } = await import("react-dom/client"));
  ({ WorkingAgentRows } = await import("./ChannelActivityPopover.tsx"));
  ({ normalizePubkey } = await import("@/shared/lib/pubkey"));
});

after(() => dom.window.close());

/**
 * Mounts the rows with a real root and returns the live DOM after UserAvatar
 * mounted its fallback (its 200ms avatar fallback delay runs the same path
 * the app runs), then unmounts so the shared now-ticker interval is cleared.
 */
async function renderRows({ agentPubkeys, agentNames, profiles }) {
  const container = dom.window.document.createElement("div");
  dom.window.document.body.appendChild(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(
      React.createElement(WorkingAgentRows, {
        activeWorking: {
          agentCount: agentPubkeys.length,
          agentPubkeys,
          ...(agentNames ? { agentNames } : {}),
          anchorAt: Date.now() - 3_000,
          channelId: "channel-id",
        },
        channelId: "channel-id",
        onOpen() {},
        profiles,
      }),
    );
  });
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 260));
  });
  const html = container.innerHTML;
  await act(async () => {
    root.unmount();
  });
  dom.window.document.body.removeChild(container);
  return html;
}

test("unnamed working agents keep distinct key-tail initials, not AN", async () => {
  const html = await renderRows({
    agentPubkeys: ["a".repeat(64), "b".repeat(64)],
  });

  assert.match(html, /Agent npub1424…rcaj/);
  assert.match(html, />RC</);
  assert.match(html, /Agent npub1hwa…04hu/);
  assert.match(html, />04</);
  assert.doesNotMatch(html, />(PN|AN)</);
});

test("authored agent names keep name-first initials", async () => {
  const roster = await renderRows({
    agentPubkeys: ["d".repeat(64)],
    agentNames: ["Helpful Bot"],
  });
  assert.match(roster, /Helpful Bot/);
  assert.match(roster, />HB</);
  assert.doesNotMatch(roster, />DM</);

  const profiled = await renderRows({
    agentPubkeys: ["e".repeat(64)],
    profiles: {
      [normalizePubkey("e".repeat(64))]: {
        avatarUrl: null,
        displayName: "Alice Example",
      },
    },
  });
  assert.match(profiled, /Alice Example/);
  assert.match(profiled, />AE</);
  assert.doesNotMatch(profiled, />NW</);
});
