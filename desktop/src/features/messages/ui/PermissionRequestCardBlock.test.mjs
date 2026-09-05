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

// The kind-9 sentinel event ID. A resolved edit must name this in
// `originalEventId` (F5 correlation).
const MESSAGE_ID =
  "deadbeef0001deadbeef0002deadbeef0003deadbeef0004deadbeef0005dead";

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
  });
}

function makeResolvedContent() {
  return JSON.stringify({
    v: 1,
    state: "resolved",
    requestNonce: "a9f3b2c1-d4e5-4f6a-b7c8-d9e0f1a2b3c4",
    originalEventId: MESSAGE_ID,
    sessionId: "sess-abc",
    turnId: "turn-xyz",
    expiresAt: FUTURE_EXPIRY,
    optionIds: ["opt-allow", "opt-deny"],
    labels: { "opt-allow": "Allow once", "opt-deny": "Deny" },
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

// Build a minimal TimelineMessage carrying the sentinel.
function makeMessage({
  content,
  signerPubkey,
  editSignerPubkey,
  ownerPubkey,
  id,
  preEditBody,
}) {
  return {
    id,
    kind: 9,
    isAgent: true,
    body: content,
    signerPubkey,
    editSignerPubkey,
    ownerPubkey,
    preEditBody,
  };
}

// The block now accepts a pre-computed `permReq` from `selectPermissionRequest`.
// We compute it here in the test helper — using the same function MessageRow uses —
// so forged signer (signer ≠ agent) fails the gate exactly as production does.
function makeIsKnownAgentPubkey(agentPubkey) {
  return (pubkey) => pubkey === agentPubkey;
}

async function renderBlock({
  content,
  signerPubkey = AGENT_PUBKEY,
  agentPubkey = AGENT_PUBKEY,
  editSignerPubkey = undefined,
  ownerPubkey = OWNER_PUBKEY,
  viewerPubkey = OWNER_PUBKEY,
  id = MESSAGE_ID,
  preEditBody = undefined,
}) {
  const { createElement, act } = await import("react");
  const { render } = await import("@testing-library/react");
  const { QueryClientProvider } = await import("@tanstack/react-query");
  const { PermissionRequestCardBlock } = await import(
    "./PermissionRequestCardBlock.tsx"
  );
  const { selectPermissionRequest } = await import(
    "./permissionRequestAuthPubkey.ts"
  );

  const qc = await makeQueryClient(viewerPubkey);
  const message = makeMessage({
    content,
    signerPubkey,
    editSignerPubkey,
    ownerPubkey,
    id,
    preEditBody,
  });
  const isKnownAgentPubkey = makeIsKnownAgentPubkey(agentPubkey);
  // Compute permReq using the same selector MessageRow uses in production.
  const permReq = selectPermissionRequest(
    message,
    isKnownAgentPubkey,
    CHANNEL_ID,
  );

  let container;
  await act(async () => {
    ({ container } = render(
      createElement(
        QueryClientProvider,
        { client: qc },
        createElement(PermissionRequestCardBlock, {
          message,
          permReq,
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
    preEditBody: makePendingContent(), // correlates nonce/session/turn ✓
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
  const { selectPermissionRequest } = await import(
    "./permissionRequestAuthPubkey.ts"
  );

  const qc = await makeQueryClient(OWNER_PUBKEY);
  const message = makeMessage({
    content: makePendingContent(EXPIRY_SECS),
    signerPubkey: AGENT_PUBKEY,
    ownerPubkey: OWNER_PUBKEY,
  });
  const permReq = selectPermissionRequest(
    message,
    makeIsKnownAgentPubkey(AGENT_PUBKEY),
    CHANNEL_ID,
  );

  let container;
  await act(async () => {
    ({ container } = render(
      createElement(
        QueryClientProvider,
        { client: qc },
        createElement(PermissionRequestCardBlock, {
          message,
          permReq,
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

// ── Delivery-outcome recovery test (Carl's P1 regression) ─────────────────────
//
// Mutation target: `permission-request-card.tsx` line
//   `if (outcome === "failed") setSubmitted(null);`
// Removing that line leaves the card permanently on "Decision sent" after a
// harness routing failure, and this test catches it while the orchestrator
// suite stays green. The test uses the `_deliveryFn` seam to control the
// outcome without a real relay.

test("test_failed_delivery_re_enables_buttons_and_successful_retry_reaches_sent", async () => {
  // Build a controllable delivery function whose outcome we resolve manually.
  // Each call returns a fresh promise; `resolveDelivery` settles the most
  // recently created one.
  let resolveDelivery;
  function makeDeliveryFn() {
    return (..._args) =>
      new Promise((resolve) => {
        resolveDelivery = resolve;
      });
  }

  const { createElement, act } = await import("react");
  const { render, fireEvent } = await import("@testing-library/react");
  const { QueryClientProvider } = await import("@tanstack/react-query");
  const { PermissionRequestCardBlock } = await import(
    "./PermissionRequestCardBlock.tsx"
  );
  const { selectPermissionRequest } = await import(
    "./permissionRequestAuthPubkey.ts"
  );

  const qc = await makeQueryClient(OWNER_PUBKEY);
  const message = makeMessage({
    content: makePendingContent(),
    signerPubkey: AGENT_PUBKEY,
    ownerPubkey: OWNER_PUBKEY,
  });
  const permReq = selectPermissionRequest(
    message,
    makeIsKnownAgentPubkey(AGENT_PUBKEY),
    CHANNEL_ID,
  );

  let container;
  await act(async () => {
    ({ container } = render(
      createElement(
        QueryClientProvider,
        { client: qc },
        createElement(PermissionRequestCardBlock, {
          message,
          permReq,
          channelId: CHANNEL_ID,
          _deliveryFn: makeDeliveryFn(),
        }),
      ),
    ));
  });

  // ── Step 1: initial render shows action buttons ──────────────────────────
  const allowBtnInitial = container.querySelector(
    '[data-testid="permission-decision-opt-allow"]',
  );
  assert.ok(allowBtnInitial !== null, "buttons present before any click");

  // ── Step 2: click — card shows "Decision sent" ────────────────────────────
  await act(async () => {
    fireEvent.click(allowBtnInitial);
  });
  assert.ok(
    container.textContent?.includes("Decision sent"),
    "card shows Decision sent after click",
  );
  assert.equal(
    container.querySelector('[data-testid="permission-decision-opt-allow"]'),
    null,
    "buttons hidden while decision is in flight",
  );

  // ── Step 3: delivery resolves "failed" → buttons must return ─────────────
  // This is Carl's regression: without `if (outcome === "failed") setSubmitted(null)`
  // the card stays stuck on "Decision sent" and the owner cannot retry.
  await act(async () => {
    resolveDelivery("failed");
    // Drain microtasks so React processes the state update.
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
  });

  const allowBtnAfterFail = container.querySelector(
    '[data-testid="permission-decision-opt-allow"]',
  );
  assert.ok(
    allowBtnAfterFail !== null,
    "buttons re-enabled after failed delivery — owner can retry",
  );
  assert.ok(
    !container.textContent?.includes("Decision sent"),
    "Decision sent text cleared after failed delivery",
  );

  // ── Step 4: retry click → delivery resolves "acked" → terminal sent ───────
  // No re-render needed. `_deliveryFn` is the closure returned by `makeDeliveryFn()`.
  // Each invocation of that closure creates a fresh promise and updates `resolveDelivery`,
  // so clicking the re-enabled button starts a new delivery loop via the same seam.
  await act(async () => {
    fireEvent.click(allowBtnAfterFail);
  });
  // Card is back to "Decision sent" for the second attempt
  assert.ok(
    container.textContent?.includes("Decision sent"),
    "Decision sent shown during second delivery attempt",
  );

  // Resolve the second delivery as "acked" → terminal state
  await act(async () => {
    resolveDelivery("acked");
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
  });

  // Buttons stay hidden — "acked" is terminal, card awaits harness kind-40003 edit
  assert.equal(
    container.querySelector('[data-testid="permission-decision-opt-allow"]'),
    null,
    "buttons stay hidden after acked — waiting for harness resolution",
  );
  assert.ok(
    container.textContent?.includes("Decision sent"),
    "Decision sent persists after acked — terminal state",
  );
});

// ── F1: reject choice renders "Denied", not "Approved" ────────────────────────
//
// Mutation target: the `chosenOptionId === allowOptionId` branch in
// `outcomeLabel` (permission-request-card.tsx). Collapsing it back to
// label-blind `Approved:` for all `applied` outcomes → the test below fails.

test("test_allow_choice_renders_approved_label", async () => {
  // optionIds[0] = "opt-allow" (allow contract). Choosing it → "Approved: Allow once".
  const resolvedContent = JSON.stringify({
    v: 1,
    state: "resolved",
    requestNonce: "a9f3b2c1-d4e5-4f6a-b7c8-d9e0f1a2b3c4",
    originalEventId: MESSAGE_ID,
    sessionId: "sess-abc",
    turnId: "turn-xyz",
    expiresAt: FUTURE_EXPIRY,
    optionIds: ["opt-allow", "opt-deny"],
    labels: { "opt-allow": "Allow once", "opt-deny": "Deny" },
    outcome: "applied",
    chosenOptionId: "opt-allow", // allow option chosen
  });

  const container = await renderBlock({
    content: resolvedContent,
    agentPubkey: AGENT_PUBKEY,
    signerPubkey: AGENT_PUBKEY,
    editSignerPubkey: AGENT_PUBKEY,
    preEditBody: makePendingContent(),
    viewerPubkey: OWNER_PUBKEY,
    ownerPubkey: OWNER_PUBKEY,
  });

  assert.ok(
    container.textContent?.includes("Approved"),
    "allow choice must render 'Approved'",
  );
  assert.ok(
    !container.textContent?.includes("Denied"),
    "allow choice must NOT render 'Denied'",
  );
});

test("test_reject_choice_renders_denied_not_approved", async () => {
  // optionIds[1] = "opt-deny" (reject contract). Choosing it → "Denied: Deny".
  // This is the F1 bug: before the fix, this rendered "Approved: Deny".
  const resolvedContent = JSON.stringify({
    v: 1,
    state: "resolved",
    requestNonce: "a9f3b2c1-d4e5-4f6a-b7c8-d9e0f1a2b3c4",
    originalEventId: MESSAGE_ID,
    sessionId: "sess-abc",
    turnId: "turn-xyz",
    expiresAt: FUTURE_EXPIRY,
    optionIds: ["opt-allow", "opt-deny"],
    labels: { "opt-allow": "Allow once", "opt-deny": "Deny" },
    outcome: "applied",
    chosenOptionId: "opt-deny", // reject option chosen
  });

  const container = await renderBlock({
    content: resolvedContent,
    agentPubkey: AGENT_PUBKEY,
    signerPubkey: AGENT_PUBKEY,
    editSignerPubkey: AGENT_PUBKEY,
    preEditBody: makePendingContent(),
    viewerPubkey: OWNER_PUBKEY,
    ownerPubkey: OWNER_PUBKEY,
  });

  assert.ok(
    container.textContent?.includes("Denied"),
    "reject choice must render 'Denied'",
  );
  assert.ok(
    !container.textContent?.includes("Approved"),
    "reject choice must NOT render 'Approved' — F1 regression proof",
  );
});

// ── F2: description renders on pending card ────────────────────────────────────

test("test_description_renders_on_pending_card", async () => {
  const contentWithDesc = JSON.stringify({
    v: 1,
    state: "pending",
    requestNonce: "a9f3b2c1-d4e5-4f6a-b7c8-d9e0f1a2b3c4",
    sessionId: "sess-abc",
    turnId: "turn-xyz",
    expiresAt: FUTURE_EXPIRY,
    optionIds: ["opt-allow", "opt-deny"],
    labels: { "opt-allow": "Allow once", "opt-deny": "Deny" },
    description: "read /etc/hosts",
  });

  const container = await renderBlock({
    content: contentWithDesc,
    viewerPubkey: OWNER_PUBKEY,
    ownerPubkey: OWNER_PUBKEY,
  });

  assert.ok(
    container.textContent?.includes("read /etc/hosts"),
    "description must appear on the pending card",
  );
});

test("test_no_description_does_not_break_pending_card", async () => {
  // No description field — card should still render with buttons
  const container = await renderBlock({
    content: makePendingContent(),
    viewerPubkey: OWNER_PUBKEY,
    ownerPubkey: OWNER_PUBKEY,
  });

  const card = container.querySelector("[data-permission-request]");
  assert.ok(card !== null, "card must render without description");

  const allowBtn = container.querySelector(
    '[data-testid="permission-decision-opt-allow"]',
  );
  assert.ok(allowBtn !== null, "buttons must render without description");
});

test("test_hostile_markup_description_renders_as_inert_text", async () => {
  // Carl's F2 requirement: hostile markup must render as inert text — no
  // element or execution, reachable by AT, not aria-hidden.
  // React renders values as text children; this test verifies by querying the
  // accessibility tree (textContent) and confirming no <script>/<img> element.
  const hostileDesc = "<script>alert('xss')</script><b>bold</b>";
  const contentWithHostile = JSON.stringify({
    v: 1,
    state: "pending",
    requestNonce: "a9f3b2c1-d4e5-4f6a-b7c8-d9e0f1a2b3c4",
    sessionId: "sess-abc",
    turnId: "turn-xyz",
    expiresAt: FUTURE_EXPIRY,
    optionIds: ["opt-allow", "opt-deny"],
    labels: { "opt-allow": "Allow once", "opt-deny": "Deny" },
    description: hostileDesc,
  });

  const container = await renderBlock({
    content: contentWithHostile,
    viewerPubkey: OWNER_PUBKEY,
    ownerPubkey: OWNER_PUBKEY,
  });

  // No <script> or <img> element created — markup treated as text
  assert.equal(
    container.querySelector("script"),
    null,
    "hostile <script> must not create a script element",
  );
  assert.equal(
    container.querySelector("b"),
    null,
    "hostile <b> must not create a bold element — markup is escaped",
  );

  // The raw string must appear as text content (HTML-escaped, reachable by AT)
  assert.ok(
    container.textContent?.includes("<script>"),
    "hostile markup must appear as literal text (accessible, not executed)",
  );
});

test("test_control_character_description_renders_safely", async () => {
  // Control characters in description must not crash the renderer or produce
  // invisible/inaccessible content.
  const controlDesc = "Run\x00command\x08with\x1fcontrol\x7fchars";
  const contentWithControl = JSON.stringify({
    v: 1,
    state: "pending",
    requestNonce: "b9f3b2c1-d4e5-4f6a-b7c8-d9e0f1a2b3c5",
    sessionId: "sess-ctrl",
    turnId: "turn-ctrl",
    expiresAt: FUTURE_EXPIRY,
    optionIds: ["opt-allow", "opt-deny"],
    labels: { "opt-allow": "Allow once", "opt-deny": "Deny" },
    description: controlDesc,
  });

  const container = await renderBlock({
    content: contentWithControl,
    viewerPubkey: OWNER_PUBKEY,
    ownerPubkey: OWNER_PUBKEY,
  });

  // Card renders (control chars do not prevent render)
  const card = container.querySelector("[data-permission-request]");
  assert.ok(card !== null, "card must render with control-char description");

  // Visible text portions are present in the accessibility tree
  assert.ok(
    container.textContent?.includes("Run"),
    "printable parts of control-char description must be in textContent",
  );
});

test("test_description_accessible_not_aria_hidden", async () => {
  // Description must be reachable by assistive technology — not wrapped in
  // aria-hidden or hidden from the accessibility tree.
  const desc = "Allow read access to /etc/hosts";
  const contentWithDesc = JSON.stringify({
    v: 1,
    state: "pending",
    requestNonce: "c9f3b2c1-d4e5-4f6a-b7c8-d9e0f1a2b3c6",
    sessionId: "sess-at",
    turnId: "turn-at",
    expiresAt: FUTURE_EXPIRY,
    optionIds: ["opt-allow", "opt-deny"],
    labels: { "opt-allow": "Allow once", "opt-deny": "Deny" },
    description: desc,
  });

  const container = await renderBlock({
    content: contentWithDesc,
    viewerPubkey: OWNER_PUBKEY,
    ownerPubkey: OWNER_PUBKEY,
  });

  // Find the element containing the description text
  const allText = container.textContent ?? "";
  assert.ok(
    allText.includes(desc),
    "description must appear in textContent (reachable by AT)",
  );

  // The description must NOT be inside an aria-hidden element
  const ariaHiddenWithDesc = [
    ...container.querySelectorAll("[aria-hidden]"),
  ].some((el) => el.textContent?.includes(desc));
  assert.equal(
    ariaHiddenWithDesc,
    false,
    "description must not be inside an aria-hidden element",
  );
});

test("test_born_resolved_no_provenance_renders_nothing", async () => {
  // A kind-9 whose body is already "resolved" but has no edit provenance
  // (no editSignerPubkey, no preEditBody). computePermissionRequest rejects
  // it — born-resolved cards bypass the agent-signed-edit requirement and
  // would render a completed card with zero proof of owner action.
  // The block returns null; selectPermissionRequest also returns null so
  // MessageRow falls back to prose (no blank row).
  const container = await renderBlock({
    content: makeResolvedContent(),
    signerPubkey: AGENT_PUBKEY,
    agentPubkey: AGENT_PUBKEY,
    editSignerPubkey: undefined, // no edit provenance
    id: MESSAGE_ID,
    preEditBody: undefined,
    viewerPubkey: OWNER_PUBKEY,
    ownerPubkey: OWNER_PUBKEY,
  });
  const card = container.querySelector("[data-permission-request]");
  assert.equal(
    card,
    null,
    "born-resolved sentinel without edit provenance must not render any card",
  );
});

test("test_correlation_mismatch_resolved_renders_nothing", async () => {
  // Resolved body where originalEventId ≠ message.id — the edit claims to
  // resolve a DIFFERENT card. computePermissionRequest rejects it.
  // The block returns null; hasPermissionRequestCard also returns false so
  // MessageRow falls back to prose (no blank row).
  const OTHER_EVENT_ID =
    "fedcba0987654321fedcba0987654321fedcba0987654321fedcba0987654321";
  const mismatchedResolved = JSON.stringify({
    v: 1,
    state: "resolved",
    requestNonce: "a9f3b2c1-d4e5-4f6a-b7c8-d9e0f1a2b3c4",
    originalEventId: OTHER_EVENT_ID, // ← names a different event
    sessionId: "sess-fixture-001",
    turnId: "turn-fixture-xyz",
    expiresAt: FUTURE_EXPIRY,
    optionIds: ["opt-allow", "opt-reject"],
    labels: { "opt-allow": "Allow once", "opt-reject": "Reject" },
    outcome: "applied",
    chosenOptionId: "opt-allow",
  });
  const container = await renderBlock({
    content: mismatchedResolved,
    signerPubkey: AGENT_PUBKEY,
    agentPubkey: AGENT_PUBKEY,
    editSignerPubkey: AGENT_PUBKEY,
    id: MESSAGE_ID, // ← MESSAGE_ID ≠ OTHER_EVENT_ID
    preEditBody: makePendingContent(),
    viewerPubkey: OWNER_PUBKEY,
    ownerPubkey: OWNER_PUBKEY,
  });
  const card = container.querySelector("[data-permission-request]");
  assert.equal(
    card,
    null,
    "correlation-mismatch resolved body must not render any card",
  );
});
