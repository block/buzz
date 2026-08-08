/**
 * Component-level render tests for `PermissionRequestCardBlock`.
 *
 * These tests verify the render-time security and behaviour gates:
 * - non-owner viewer sees read-only card (no buttons)
 * - forged signer (signerPubkey ≠ agentPubkey) renders nothing
 * - agent-signed edit resolves the card to non-actionable state
 * - owner/attacker-signed edits do NOT resolve the card
 * - expiry: buttons disabled after the ticking clock crosses expiresAt
 *
 * Wire contract: harness signs bare JSON as the kind:9 event content.
 */
import assert from "node:assert/strict";
import { after, afterEach, before, mock, test } from "node:test";

import { JSDOM } from "jsdom";

// ── jsdom setup ───────────────────────────────────────────────────────────────

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

// Use fake timers for all tests: prevents real setIntervals in
// PendingPermissionRequestCard from keeping the event loop alive after unmount.
// All tests use a fixed epoch so `Date.now()` returns a deterministic value.
const FAKE_NOW_MS = 1_000_000_000_000; // far from real time — avoids expiry surprises

before(() => {
  // Enable fake timers before any components load so Date.now() is stable.
  mock.timers.enable({ apis: ["setInterval", "Date"], now: FAKE_NOW_MS });

  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
    // smoothCorners.ts requires MutationObserver; ResizeObserver used by
    // various attachment components. Provide no-op stubs.
    MutationObserver: class {
      observe() {}
      disconnect() {}
      takeRecords() {
        return [];
      }
    },
    ResizeObserver: class {
      observe() {}
      unobserve() {}
      disconnect() {}
    },
  });
  dom.window.matchMedia = () => ({
    matches: false,
    addEventListener() {},
    removeEventListener() {},
  });
  // smoothCorners.ts attaches a MutationObserver to the document; stub on window too
  dom.window.MutationObserver = globalThis.MutationObserver;
  dom.window.ResizeObserver = globalThis.ResizeObserver;
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
  if (sharedQc) {
    sharedQc.clear();
    sharedQc = undefined;
  }
  // Drain any pending fake timers from this test before the next one starts.
  mock.timers.reset();
  mock.timers.enable({ apis: ["setInterval", "Date"], now: FAKE_NOW_MS });
});

after(async () => {
  mock.timers.reset();
  dom.window.close();
});

// ── Fixtures ──────────────────────────────────────────────────────────────────

const AGENT_PUBKEY =
  "aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899";
const OWNER_PUBKEY =
  "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const ATTACKER_PUBKEY =
  "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
const CHANNEL_ID = "test-channel-id";

// Unix epoch far in the future — buttons are live under the fake clock
const FUTURE_EXPIRY = Math.floor(FAKE_NOW_MS / 1000) + 9_999_999;
// Unix epoch in the past — buttons expired immediately (prefixed _ = intentionally unused)
const _PAST_EXPIRY = 1;

function makePendingContent(expiresAt = FUTURE_EXPIRY) {
  return JSON.stringify({
    v: 1,
    state: "pending",
    requestNonce: "a9f3b2c1-d4e5-4f6a-b7c8-d9e0f1a2b3c4",
    sessionId: "sess-abc",
    turnId: "turn-xyz",
    expiresAt,
    optionIds: ["opt-allow", "opt-deny"],
    labels: { "opt-allow": "Allow once", "opt-deny": "Deny" },
    hasDurableRule: false,
    durableRuleNote: null,
  });
}

function makeResolvedContent() {
  return JSON.stringify({
    v: 1,
    state: "resolved",
    requestNonce: "a9f3b2c1-d4e5-4f6a-b7c8-d9e0f1a2b3c4",
    originalEventId:
      "deadbeef0001deadbeef0002deadbeef0003deadbeef0004deadbeef0005dead",
    sessionId: "sess-abc",
    turnId: "turn-xyz",
    expiresAt: FUTURE_EXPIRY,
    optionIds: ["opt-allow", "opt-deny"],
    labels: { "opt-allow": "Allow once", "opt-deny": "Deny" },
    hasDurableRule: false,
    durableRuleNote: null,
    outcome: "applied",
    chosenOptionId: "opt-allow",
  });
}

// Shared QueryClient — created once, cleared between tests.
// `gcTime: 0` prevents React Query's garbage-collection timer from keeping
// the event loop alive after the test completes.
let sharedQc;

async function getQueryClient(viewerPubkey) {
  const { QueryClient } = await import("@tanstack/react-query");
  if (sharedQc) sharedQc.clear();
  sharedQc = new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: 0, staleTime: Infinity },
    },
  });
  sharedQc.setQueryData(["identity"], { pubkey: viewerPubkey });
  return sharedQc;
}

async function makeQueryClient(viewerPubkey) {
  return getQueryClient(viewerPubkey);
}

// ── Render helper ─────────────────────────────────────────────────────────────

async function renderBlock({
  content,
  signerPubkey = AGENT_PUBKEY,
  agentPubkey = AGENT_PUBKEY,
  editSignerPubkey = undefined,
  ownerPubkey = OWNER_PUBKEY,
  viewerPubkey = OWNER_PUBKEY,
}) {
  const { createElement, act } = await import("react");
  const { render } = await import("@testing-library/react");
  const { QueryClientProvider } = await import("@tanstack/react-query");
  const { PermissionRequestCardBlock } = await import(
    "./PermissionRequestCardBlock.tsx"
  );

  const qc = await makeQueryClient(viewerPubkey);

  let container;
  await act(async () => {
    ({ container } = render(
      createElement(
        QueryClientProvider,
        { client: qc },
        createElement(PermissionRequestCardBlock, {
          content,
          interactive: true,
          agentPubkey,
          signerPubkey,
          editSignerPubkey,
          ownerPubkey,
          channelId: CHANNEL_ID,
        }),
      ),
    ));
  });

  return container;
}

// ── Tests ─────────────────────────────────────────────────────────────────────

test("test_owner_viewer_sees_action_buttons_on_pending_card", async () => {
  const container = await renderBlock({
    content: makePendingContent(),
    viewerPubkey: OWNER_PUBKEY,
    ownerPubkey: OWNER_PUBKEY,
  });

  const allowBtn = container.querySelector(
    '[data-testid="permission-decision-opt-allow"]',
  );
  const denyBtn = container.querySelector(
    '[data-testid="permission-decision-opt-deny"]',
  );
  assert.ok(allowBtn !== null, "owner should see allow button");
  assert.ok(denyBtn !== null, "owner should see deny button");
});

test("test_non_owner_viewer_sees_read_only_card_no_buttons", async () => {
  const container = await renderBlock({
    content: makePendingContent(),
    viewerPubkey: ATTACKER_PUBKEY, // not the owner
    ownerPubkey: OWNER_PUBKEY,
  });

  // Card should render (sentinel parsed and agent matches signer)
  const card = container.querySelector("[data-permission-request]");
  assert.ok(card !== null, "card renders for non-owner");

  // But no action buttons
  const btn = container.querySelector('[data-testid^="permission-decision-"]');
  assert.equal(btn, null, "non-owner must not see action buttons");

  // Read-only indicator text present
  assert.ok(
    container.textContent?.includes("Waiting for owner approval"),
    "non-owner sees waiting message",
  );
});

test("test_forged_signer_renders_nothing", async () => {
  const container = await renderBlock({
    content: makePendingContent(),
    agentPubkey: AGENT_PUBKEY,
    signerPubkey: ATTACKER_PUBKEY, // signer ≠ agent → rejected
    viewerPubkey: OWNER_PUBKEY,
    ownerPubkey: OWNER_PUBKEY,
  });

  const card = container.querySelector("[data-permission-request]");
  assert.equal(card, null, "forged signer must not render any card");
});

test("test_agent_signed_edit_resolves_card_to_non_actionable", async () => {
  // kind-40003 edit signed by the original agent → resolved card, no buttons
  const container = await renderBlock({
    content: makeResolvedContent(),
    agentPubkey: AGENT_PUBKEY,
    signerPubkey: AGENT_PUBKEY,
    editSignerPubkey: AGENT_PUBKEY, // edit signed by agent ✓
    viewerPubkey: OWNER_PUBKEY,
    ownerPubkey: OWNER_PUBKEY,
  });

  const card = container.querySelector("[data-permission-request]");
  assert.ok(card !== null, "resolved card renders");

  const btn = container.querySelector('[data-testid^="permission-decision-"]');
  assert.equal(btn, null, "resolved card has no action buttons");

  assert.ok(
    container.textContent?.includes("Permission request resolved"),
    "resolved label present",
  );
});

test("test_owner_signed_edit_does_not_resolve_card", async () => {
  // kind-40003 signed by owner, not agent → edit-authenticity gate rejects
  const container = await renderBlock({
    content: makeResolvedContent(),
    agentPubkey: AGENT_PUBKEY,
    signerPubkey: AGENT_PUBKEY,
    editSignerPubkey: OWNER_PUBKEY, // edit signed by owner ✗
    viewerPubkey: OWNER_PUBKEY,
    ownerPubkey: OWNER_PUBKEY,
  });

  // computePermissionRequest returns null → block renders nothing
  const card = container.querySelector("[data-permission-request]");
  assert.equal(card, null, "owner-signed edit must not resolve card");
});

test("test_attacker_signed_edit_does_not_resolve_card", async () => {
  const container = await renderBlock({
    content: makeResolvedContent(),
    agentPubkey: AGENT_PUBKEY,
    signerPubkey: AGENT_PUBKEY,
    editSignerPubkey: ATTACKER_PUBKEY, // attacker edit ✗
    viewerPubkey: OWNER_PUBKEY,
    ownerPubkey: OWNER_PUBKEY,
  });

  const card = container.querySelector("[data-permission-request]");
  assert.equal(card, null, "attacker-signed edit must not resolve card");
});

test("test_expiry_disables_buttons_after_clock_tick", async () => {
  // FAKE_NOW_MS is the current epoch. Set expiry to 1s in the future.
  const EXPIRY_SECS = Math.floor(FAKE_NOW_MS / 1000) + 1;

  const { createElement, act } = await import("react");
  const { render } = await import("@testing-library/react");
  const { QueryClientProvider } = await import("@tanstack/react-query");
  const { PermissionRequestCardBlock } = await import(
    "./PermissionRequestCardBlock.tsx"
  );

  const qc = await makeQueryClient(OWNER_PUBKEY);

  let container;
  await act(async () => {
    ({ container } = render(
      createElement(
        QueryClientProvider,
        { client: qc },
        createElement(PermissionRequestCardBlock, {
          content: makePendingContent(EXPIRY_SECS),
          interactive: true,
          agentPubkey: AGENT_PUBKEY,
          signerPubkey: AGENT_PUBKEY,
          ownerPubkey: OWNER_PUBKEY,
          channelId: CHANNEL_ID,
        }),
      ),
    ));
  });

  // Before expiry: buttons must be present
  const btnBefore = container.querySelector(
    '[data-testid="permission-decision-opt-allow"]',
  );
  assert.ok(btnBefore !== null, "buttons present before expiry");

  // Advance clock by 2 seconds — past the 1s expiry
  await act(async () => {
    mock.timers.tick(2_000);
  });

  // After expiry: buttons must be gone, timed-out message shown
  const btnAfter = container.querySelector(
    '[data-testid="permission-decision-opt-allow"]',
  );
  assert.equal(btnAfter, null, "buttons absent after expiry tick");
  assert.ok(
    container.textContent?.includes("Timed out"),
    "timed-out message shown after expiry",
  );
});
