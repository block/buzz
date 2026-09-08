/**
 * Mounted renderer regressions for the hosted-communities account identity
 * display (P2: one authoritative hosted identity).
 *
 * The Builderlab IPC response carries the account identity as two
 * independent fields — `pubkey_hex` and `npub` — and nothing on the path
 * proves they encode the same key. The card's binding decisions (the
 * mismatch gate, switch-to-device, hosted-community operations) all act on
 * `pubkey_hex`, so the displayed account npub must be derived from that same
 * authoritative key. These tests mount the real card through the real
 * providers and drive only the Tauri IPC boundary: two individually valid
 * but contradictory fields must leave the rendered npub following
 * `pubkey_hex`, never the server's `npub` spelling, and an unusable
 * authoritative hex must fall back to the neutral label rather than the
 * unverified hosted npub.
 *
 * Mutation proof: restore a display preference for `identity.npub` (e.g.
 * `const hostedNpub = identity?.npub ? canonicalNpub(identity.npub) : null`
 * rendered ahead of the hex-derived npub) and the contradictory-field tests
 * go RED — the mismatch row and the connected row render the other key.
 */

import assert from "node:assert/strict";
import { after, afterEach, before, describe, it } from "node:test";
import { JSDOM } from "jsdom";
import { npubEncode } from "nostr-tools/nip19";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

Object.assign(globalThis, {
  HTMLElement: dom.window.HTMLElement,
  HTMLIFrameElement: dom.window.HTMLIFrameElement,
  IS_REACT_ACT_ENVIRONMENT: true,
  MutationObserver: dom.window.MutationObserver,
  ResizeObserver: class {
    observe() {}
    unobserve() {}
    disconnect() {}
  },
  document: dom.window.document,
  localStorage: dom.window.localStorage,
  self: dom.window,
  window: dom.window,
});
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  value: dom.window.navigator,
});
dom.window.requestAnimationFrame = (callback) => setTimeout(callback, 0);
globalThis.requestAnimationFrame = dom.window.requestAnimationFrame;
dom.window.ResizeObserver = globalThis.ResizeObserver;
dom.window.matchMedia ??= (query) => ({
  matches: false,
  media: query,
  onchange: null,
  addListener: () => {},
  removeListener: () => {},
  addEventListener: () => {},
  removeEventListener: () => {},
  dispatchEvent: () => false,
});
globalThis.matchMedia = dom.window.matchMedia;
// Copy DOM-level globals JSDOM defines without a `window.` prefix that Radix
// and icon components reference directly (see
// HarnessCatalogDialog.acpForcedGate.test.mjs for the proven template).
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
        "TouchEvent",
        "WheelEvent",
        "EventTarget",
        "Text",
        "Comment",
        "DocumentFragment",
        "Range",
        "Selection",
        "getComputedStyle",
        "IntersectionObserver",
        "ResizeObserver",
      ].includes(key))
  ) {
    const val = dom.window[key];
    if (val !== undefined) globalThis[key] = val;
  }
}
// getComputedStyle must be bound to dom.window or it throws "Illegal invocation".
globalThis.getComputedStyle = dom.window.getComputedStyle.bind(dom.window);
// Radix layer coordination dispatches plain objects; drop them (template).
const _origDispatch = dom.window.EventTarget.prototype.dispatchEvent;
dom.window.EventTarget.prototype.dispatchEvent = function (event) {
  if (!(event instanceof dom.window.Event)) return false;
  return _origDispatch.call(this, event);
};
globalThis.EventTarget = dom.window.EventTarget;

// ── Keys under test ────────────────────────────────────────────────────────────

/** The authoritative account key, exactly as the reviewer's scenario spells it. */
const ACCOUNT_HEX = "a".repeat(64);
/** A different, individually valid key the contradictory server npub encodes. */
const OTHER_HEX = "b".repeat(64);
/** The local device key for the mismatch scenario. */
const DEVICE_HEX = "c".repeat(64);
const ACCOUNT_NPUB = npubEncode(ACCOUNT_HEX);
const OTHER_NPUB = npubEncode(OTHER_HEX);
const DEVICE_NPUB = npubEncode(DEVICE_HEX);

// ── Tauri IPC stub ────────────────────────────────────────────────────────────

let builderlabIdentity;
let deviceRawIdentity;

globalThis.__TAURI_INTERNALS__ = {
  invoke: (command) => {
    switch (command) {
      case "get_builderlab_auth":
        return Promise.resolve({
          email: "owner@example.com",
          name: "Owner",
          expiresAt: "2099-01-01T00:00:00Z",
        });
      case "get_builderlab_nostr_identity":
        return Promise.resolve(
          builderlabIdentity
            ? { identity: builderlabIdentity }
            : { error: { code: "missing_mapping", setup_needed: true } },
        );
      case "list_builderlab_communities":
        return Promise.resolve({ communities: [] });
      case "get_identity":
        return Promise.resolve(deviceRawIdentity);
      default:
        return Promise.reject(new Error(`unmocked: ${command}`));
    }
  },
  transformCallback: () => 1,
};
dom.window.__TAURI_INTERNALS__ = globalThis.__TAURI_INTERNALS__;

// ── Deferred imports ──────────────────────────────────────────────────────────

let React,
  act,
  render,
  screen,
  cleanup,
  QueryClient,
  QueryClientProvider,
  HostedCommunitiesSettingsCard,
  CommunitiesProvider,
  CommunityOnboardingProvider;

before(async () => {
  ({ default: React, act } = await import("react"));
  ({ render, screen, cleanup } = await import("@testing-library/react"));
  ({ QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  ));
  ({ HostedCommunitiesSettingsCard } = await import(
    "./HostedCommunitiesSettingsCard.tsx"
  ));
  ({ CommunitiesProvider } = await import(
    "@/features/communities/useCommunities.tsx"
  ));
  ({ CommunityOnboardingProvider } = await import(
    "@/features/onboarding/communityOnboarding.tsx"
  ));
});

afterEach(() => {
  cleanup();
});

after(() => dom.window.close());

// ── Harness ───────────────────────────────────────────────────────────────────

/** Mount the real card with the providers AppShell composes for it. */
async function mountCard() {
  const queryClient = new QueryClient({
    defaultOptions: {
      // gcTime 0 matches the mounted-consumer pattern in acpRuntimesQuery:
      // the card's identity query goes inactive on cleanup, and the default
      // 5-minute gc timer would otherwise idle the test runner for its full
      // duration after the assertions complete.
      queries: { retry: false, gcTime: 0 },
    },
  });
  render(
    React.createElement(
      QueryClientProvider,
      { client: queryClient },
      React.createElement(
        CommunitiesProvider,
        null,
        React.createElement(
          CommunityOnboardingProvider,
          null,
          React.createElement(HostedCommunitiesSettingsCard),
        ),
      ),
    ),
  );
  // The card chains get_builderlab_auth → loadAccount (identity +
  // communities) and useIdentityQuery resolves get_identity; flush them.
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 50));
  });
}

/** The <dd> rendered beside a <dt> label (the mismatch rows are a <dl>). */
function valueForLabel(label) {
  const dt = screen.getByText(label, { exact: true });
  const dd = dt.nextElementSibling;
  assert.ok(dd, `expected a value cell after the "${label}" label`);
  return dd.textContent ?? "";
}

/** The npub span in the connected row ("Buzz identity connected …npub"). */
function connectedIdentityText() {
  const row = screen.getByText("Buzz identity connected", { exact: true });
  const span = row.querySelector("span.font-mono");
  assert.ok(span, "expected the connected row to show the identity npub");
  return span.textContent ?? "";
}

// ── Tests ─────────────────────────────────────────────────────────────────────

describe("HostedCommunitiesSettingsCard account identity display — P2 regression (mounted card)", () => {
  it("shows the hex-derived account npub when the hosted npub encodes a different valid key (mismatch state)", async () => {
    builderlabIdentity = { pubkey_hex: ACCOUNT_HEX, npub: OTHER_NPUB };
    deviceRawIdentity = { pubkey: DEVICE_HEX, display_name: "Local device" };

    await mountCard();

    assert.ok(
      screen.getByText(
        "This account is connected to a different Buzz identity",
      ),
      "contradictory bound key vs device key must surface the mismatch state",
    );
    assert.equal(
      valueForLabel("Account uses"),
      ACCOUNT_NPUB,
      "the account row must display the authoritative pubkey_hex's npub",
    );
    assert.equal(
      valueForLabel("This device"),
      DEVICE_NPUB,
      "the device row must display the local key's npub",
    );
    assert.ok(
      !document.body.textContent.includes(OTHER_NPUB),
      "the contradictory hosted npub must never be rendered",
    );
  });

  it("shows the hex-derived account npub when the hosted npub encodes a different valid key (connected state)", async () => {
    builderlabIdentity = { pubkey_hex: ACCOUNT_HEX, npub: OTHER_NPUB };
    deviceRawIdentity = { pubkey: ACCOUNT_HEX, display_name: "Local device" };

    await mountCard();

    assert.equal(
      connectedIdentityText(),
      ACCOUNT_NPUB,
      "the connected row must display the authoritative pubkey_hex's npub",
    );
    assert.ok(
      !document.body.textContent.includes(OTHER_NPUB),
      "the contradictory hosted npub must never be rendered",
    );
  });

  it("displays the account npub unchanged when both fields agree", async () => {
    builderlabIdentity = { pubkey_hex: ACCOUNT_HEX, npub: ACCOUNT_NPUB };
    deviceRawIdentity = { pubkey: ACCOUNT_HEX, display_name: "Local device" };

    await mountCard();

    assert.equal(
      connectedIdentityText(),
      ACCOUNT_NPUB,
      "a consistent hosted identity must still render its canonical npub",
    );
  });

  it("falls back to the neutral label when the authoritative hex is unusable but the hosted npub is valid", async () => {
    // 63 chars: valid hex alphabet, non-identity length — unusable as a key.
    builderlabIdentity = { pubkey_hex: "a".repeat(63), npub: OTHER_NPUB };
    deviceRawIdentity = { pubkey: DEVICE_HEX, display_name: "Local device" };

    await mountCard();

    assert.ok(
      screen.getByText(
        "This account is connected to a different Buzz identity",
      ),
      "an unusable bound key must still surface the mismatch state",
    );
    assert.equal(
      valueForLabel("Account uses"),
      "Unavailable",
      "an unusable authoritative hex must render the neutral label",
    );
    assert.ok(
      !document.body.textContent.includes(OTHER_NPUB),
      "the unverified hosted npub must not stand in for the unusable hex",
    );
  });
});
