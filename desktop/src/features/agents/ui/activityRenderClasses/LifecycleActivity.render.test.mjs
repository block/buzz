import assert from "node:assert/strict";
import { after, afterEach, before, mock, test } from "node:test";

import { JSDOM } from "jsdom";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { LifecycleActivity } from "./LifecycleActivity.tsx";
import { buildTranscript } from "../agentSessionTranscript.ts";

// ---------------------------------------------------------------------------
// Shared fixtures
// ---------------------------------------------------------------------------

const BASE_PROPS = {
  agentAvatarUrl: null,
  agentName: "Test Agent",
  agentPubkey: "pubkey123",
};

const BASE_IDENTITY = {
  turnId: "turn-1",
  sessionId: "session-1",
  channelId: "channel-1",
};

/**
 * Build a pending permission lifecycle item with the given options array.
 * The card is actionable (awaiting a user decision) and has a request nonce.
 */
function pendingPermissionItem(options) {
  return {
    id: "perm-1",
    type: "lifecycle",
    renderClass: "permission",
    title: "Tool requires approval",
    text: "Run shell command",
    timestamp: "2026-08-10T00:00:00.000Z",
    requestNonce: "nonce-abc",
    actionable: true,
    options,
    ...BASE_IDENTITY,
  };
}

// ---------------------------------------------------------------------------
// jsdom + fake-timer setup (required for the interactive delivery-seam tests)
// The static renderToStaticMarkup tests do not use document/window but the
// setup is harmless for them: it only assigns globals they never read.
// ---------------------------------------------------------------------------

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

// Deterministic wall-clock epoch — far from real time to avoid expiry surprises.
// expiresAt is set to FAKE_NOW_SECS + 9_999_999 in the interactive tests so
// the card never expires during the test.
const FAKE_NOW_MS = 1_000_000_000_000;

before(() => {
  mock.timers.enable({ apis: ["setInterval", "Date"], now: FAKE_NOW_MS });

  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
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
  dom.window.MutationObserver = globalThis.MutationObserver;
  dom.window.ResizeObserver = globalThis.ResizeObserver;
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
  mock.timers.reset();
  mock.timers.enable({ apis: ["setInterval", "Date"], now: FAKE_NOW_MS });
});

after(() => {
  mock.timers.reset();
  dom.window.close();
});

test("test_allow_once_renders_actionable_allow_button", () => {
  const html = renderToStaticMarkup(
    React.createElement(LifecycleActivity, {
      ...BASE_PROPS,
      item: pendingPermissionItem([
        { optionId: "opt-allow", kind: "allow_once", label: "Allow once" },
      ]),
    }),
  );

  // The button must be present and labelled correctly.
  assert.ok(
    html.includes("permission-decision-opt-allow"),
    "allow_once option should render a button with its optionId testid",
  );
  assert.ok(
    html.includes("Allow once"),
    "allow_once option should show its label",
  );

  // The persistent-grant badge must NOT appear for a pure allow_once card.
  assert.ok(
    !html.includes("permission-decision-persistent-grant"),
    "allow_once card should not render the persistent-grant badge",
  );
});

// ---------------------------------------------------------------------------
// reject_once — renders a red actionable Deny button
// ---------------------------------------------------------------------------

test("test_reject_once_renders_actionable_deny_button", () => {
  const html = renderToStaticMarkup(
    React.createElement(LifecycleActivity, {
      ...BASE_PROPS,
      item: pendingPermissionItem([
        { optionId: "opt-deny", kind: "reject_once" },
      ]),
    }),
  );

  assert.ok(
    html.includes("permission-decision-opt-deny"),
    "reject_once option should render a button with its optionId testid",
  );
  // Deny button uses destructive styling; verify at least the testid is there.
  assert.ok(
    !html.includes("permission-decision-persistent-grant"),
    "reject_once card should not render the persistent-grant badge",
  );
});

// ---------------------------------------------------------------------------
// allow_always — not actionable, no badge (F3: persistent-grant badge removed)
// ---------------------------------------------------------------------------

test("test_allow_always_renders_no_button_and_no_badge", () => {
  // After F3: allow_always is NOT in ACTIONABLE_KINDS and the persistent-grant
  // badge has been removed. A card with only allow_always renders nothing
  // actionable — no button and no badge — because the two-option contract
  // (allow_once / reject_once only) is enforced at both the Rust sentinel and
  // the observer surface.
  const html = renderToStaticMarkup(
    React.createElement(LifecycleActivity, {
      ...BASE_PROPS,
      item: pendingPermissionItem([
        { optionId: "opt-always", kind: "allow_always", label: "Always allow" },
      ]),
    }),
  );

  // No button for allow_always.
  assert.ok(
    !html.includes("permission-decision-opt-always"),
    "allow_always option must not render an actionable button",
  );
  // No <button> element at all — no actionable options.
  assert.ok(
    !html.includes("<button"),
    "allow_always-only card must not render any button element",
  );
  // The persistent-grant badge is gone — it was the only surface that showed
  // allow_always and it has been removed in F3.
  assert.ok(
    !html.includes("permission-decision-persistent-grant"),
    "persistent-grant badge must not render after F3 removal",
  );
  assert.ok(
    !html.includes("Permanent grant"),
    "persistent-grant copy must not render after F3 removal",
  );
});

// ---------------------------------------------------------------------------
// Unknown kind — fail closed: renders nothing actionable, no badge
// ---------------------------------------------------------------------------

test("test_unknown_kind_fails_closed_renders_nothing", () => {
  const html = renderToStaticMarkup(
    React.createElement(LifecycleActivity, {
      ...BASE_PROPS,
      item: pendingPermissionItem([
        { optionId: "opt-mystery", kind: "future_unknown_verb" },
      ]),
    }),
  );

  // No button for the unknown kind.
  assert.ok(
    !html.includes("permission-decision-opt-mystery"),
    "unknown kind must not render an actionable button",
  );
  // No persistent-grant badge either.
  assert.ok(
    !html.includes("permission-decision-persistent-grant"),
    "unknown kind must not render the persistent-grant badge",
  );
  // No button element at all.
  assert.ok(
    !html.includes("<button"),
    "unknown-kind-only card must not render any button element",
  );
  // The outer permission card shell is still rendered (title row etc.).
  assert.ok(
    html.includes("transcript-permission-item"),
    "unknown kind still renders the permission card shell",
  );
});

// ---------------------------------------------------------------------------
// Unknown reject_*-prefixed kind — fail closed: exact allowlist, not prefix
// ---------------------------------------------------------------------------

test("test_unknown_reject_prefixed_kind_fails_closed_renders_nothing", () => {
  const html = renderToStaticMarkup(
    React.createElement(LifecycleActivity, {
      ...BASE_PROPS,
      item: pendingPermissionItem([
        { optionId: "opt-reject-future", kind: "reject_later_v2" },
      ]),
    }),
  );

  // A reject-prefixed but unrecognized kind must NOT render a trusted button:
  // recognition is an exact allowlist, not a prefix match.
  assert.ok(
    !html.includes("permission-decision-opt-reject-future"),
    "unknown reject_*-prefixed kind must not render an actionable button",
  );
  assert.ok(
    !html.includes("<button"),
    "unknown reject_*-prefixed-only card must not render any button element",
  );
  assert.ok(
    !html.includes("permission-decision-persistent-grant"),
    "unknown reject_*-prefixed kind must not render the persistent-grant badge",
  );
  // The outer permission card shell is still rendered.
  assert.ok(
    html.includes("transcript-permission-item"),
    "unknown reject_*-prefixed kind still renders the permission card shell",
  );
});

// ---------------------------------------------------------------------------
// reject_always — not actionable (F3: removed from ACTIONABLE_KINDS)
// ---------------------------------------------------------------------------

test("test_reject_always_renders_no_button", () => {
  // After F3: reject_always is removed from ACTIONABLE_KINDS. The thread card
  // cannot grant permanent denial; the ACP read loop accepts only allow_once and
  // reject_once. A reject_always option must render as inert — no clickable
  // button. The outer card shell is still rendered (the activity still appears
  // in the transcript), but no action can be taken on it.
  const html = renderToStaticMarkup(
    React.createElement(LifecycleActivity, {
      ...BASE_PROPS,
      item: pendingPermissionItem([
        { optionId: "opt-reject-always", kind: "reject_always" },
      ]),
    }),
  );

  // Must NOT render a clickable button for reject_always.
  assert.ok(
    !html.includes("permission-decision-opt-reject-always"),
    "reject_always must not render an actionable button after F3",
  );
  // No button element at all — no actionable options present.
  assert.ok(
    !html.includes("<button"),
    "reject_always-only card must not render any button element after F3",
  );
  // No persistent-grant badge either.
  assert.ok(
    !html.includes("permission-decision-persistent-grant"),
    "reject_always card must not render the persistent-grant badge",
  );
  // The outer card shell IS rendered.
  assert.ok(
    html.includes("transcript-permission-item"),
    "reject_always still renders the outer permission card shell",
  );
});

// ---------------------------------------------------------------------------
// Mixed options — allow_once + allow_always: only allow_once actionable
// No persistent-grant badge after F3 removal
// ---------------------------------------------------------------------------

test("test_mixed_allow_once_and_allow_always_only_allow_once_actionable", () => {
  // After F3: allow_always is not in ACTIONABLE_KINDS and the persistent-grant
  // badge is removed. A mixed card renders only the allow_once button; allow_always
  // is inert context with no UI surface.
  const html = renderToStaticMarkup(
    React.createElement(LifecycleActivity, {
      ...BASE_PROPS,
      item: pendingPermissionItem([
        { optionId: "opt-once", kind: "allow_once" },
        { optionId: "opt-always", kind: "allow_always" },
      ]),
    }),
  );

  // allow_once produces a button.
  assert.ok(
    html.includes("permission-decision-opt-once"),
    "allow_once in mixed card should render a button",
  );
  // allow_always does NOT produce a button.
  assert.ok(
    !html.includes("permission-decision-opt-always"),
    "allow_always in mixed card must not render a button",
  );
  // No persistent-grant badge — it has been removed.
  assert.ok(
    !html.includes("permission-decision-persistent-grant"),
    "mixed card must not render the persistent-grant badge after F3 removal",
  );
  assert.ok(
    !html.includes("Permanent grant"),
    "mixed card must not render persistent-grant copy after F3 removal",
  );
});

// ---------------------------------------------------------------------------
// F3 contract: all four adapter option kinds — only allow_once + reject_once
// are actionable; allow_always and reject_always are inert.
// ---------------------------------------------------------------------------

test("test_four_option_contract_only_allow_once_and_reject_once_actionable", () => {
  // The two-option contract: the thread card may only action allow_once and
  // reject_once. This test covers all four recognized adapter option kinds in
  // a single card and verifies the exact set of rendered buttons.
  //
  // Mutation proof: removing "reject_once" from ACTIONABLE_KINDS in
  // LifecycleActivity.tsx makes "permission-decision-opt-deny" absent — the
  // assertion on opt-deny goes red. Removing "allow_once" makes opt-allow
  // absent similarly. The ACP read loop on the Rust side accepts only the
  // two option IDs snapshotted into CardActions (allow_once / reject_once);
  // sending an allow_always or reject_always option ID is silently ignored.
  const html = renderToStaticMarkup(
    React.createElement(LifecycleActivity, {
      ...BASE_PROPS,
      item: pendingPermissionItem([
        { optionId: "opt-allow", kind: "allow_once", label: "Allow once" },
        { optionId: "opt-deny", kind: "reject_once", label: "Deny" },
        { optionId: "opt-always", kind: "allow_always", label: "Always allow" },
        {
          optionId: "opt-reject-always",
          kind: "reject_always",
          label: "Always deny",
        },
      ]),
    }),
  );

  // Only allow_once and reject_once render buttons.
  assert.ok(
    html.includes("permission-decision-opt-allow"),
    "allow_once must render an actionable button",
  );
  assert.ok(
    html.includes("permission-decision-opt-deny"),
    "reject_once must render an actionable button",
  );

  // allow_always and reject_always must NOT render buttons.
  assert.ok(
    !html.includes("permission-decision-opt-always"),
    "allow_always must not render a button in a mixed four-option card",
  );
  assert.ok(
    !html.includes("permission-decision-opt-reject-always"),
    "reject_always must not render a button in a mixed four-option card",
  );

  // No persistent-grant badge — removed in F3.
  assert.ok(
    !html.includes("permission-decision-persistent-grant"),
    "four-option card must not render the persistent-grant badge",
  );
  // Exactly two <button> elements (allow_once + reject_once).
  const buttonCount = (html.match(/<button/g) ?? []).length;
  assert.equal(
    buttonCount,
    2,
    `four-option card must render exactly 2 buttons (allow_once + reject_once); got ${buttonCount}`,
  );
});

// ---------------------------------------------------------------------------
// F3 cross-layer: acp_read → buildTranscript → LifecycleActivity
//
// Starts with all four adapter option kinds in the request payload.
// Drives the event through the full transcript reducer so the card is built
// from the real processing path, not a hand-rolled fixture.
// Then renders via LifecycleActivity and confirms the two-button contract.
// ---------------------------------------------------------------------------

test("test_f3_cross_layer_four_options_acp_read_to_lifecycle_activity_two_buttons", () => {
  // Build an acp_read event carrying all four adapter option kinds.
  // This is the real wire shape the observer feed emits when the agent
  // requests permission with a full four-option set.
  const acpReadEvent = {
    seq: 1,
    timestamp: "2026-09-01T10:00:00.000Z",
    kind: "acp_read",
    agentIndex: 0,
    channelId: "ch-f3-cross",
    sessionId: "sess-f3-cross",
    turnId: "turn-f3-cross",
    payload: {
      jsonrpc: "2.0",
      id: "req-f3",
      method: "session/request_permission",
      params: {
        title: "Tool requires approval",
        toolCallId: "tc-f3",
        // Four option kinds offered by the adapter.
        options: [
          {
            optionId: "opt-allow-once",
            kind: "allow_once",
            name: "Allow once",
          },
          { optionId: "opt-reject-once", kind: "reject_once", name: "Deny" },
          {
            optionId: "opt-allow-always",
            kind: "allow_always",
            name: "Always allow",
          },
          {
            optionId: "opt-reject-always",
            kind: "reject_always",
            name: "Always deny",
          },
        ],
      },
    },
    // Authorization envelope: marks the card as actionable with a nonce.
    authorization: {
      requestNonce: "nonce-f3-cross",
      actionable: true,
    },
  };

  // 1. Drive through the transcript reducer.
  const transcript = buildTranscript([acpReadEvent]);
  const card = transcript.find((item) => item.renderClass === "permission");
  assert.ok(card, "transcript must contain a permission card");
  assert.equal(
    card.requestNonce,
    "nonce-f3-cross",
    "card must carry the request nonce",
  );
  assert.ok(card.actionable, "card must be actionable");
  assert.ok(Array.isArray(card.options), "card must have options");
  assert.equal(card.options.length, 4, "all four options must be on the card");

  // 2. Render via LifecycleActivity and assert the two-button contract.
  const html = renderToStaticMarkup(
    React.createElement(LifecycleActivity, {
      ...BASE_PROPS,
      item: card,
    }),
  );

  // Only allow_once and reject_once render buttons (ACTIONABLE_KINDS contract).
  assert.ok(
    html.includes("permission-decision-opt-allow-once"),
    "allow_once must render a button via cross-layer path",
  );
  assert.ok(
    html.includes("permission-decision-opt-reject-once"),
    "reject_once must render a button via cross-layer path",
  );

  // allow_always and reject_always must NOT render buttons.
  assert.ok(
    !html.includes("permission-decision-opt-allow-always"),
    "allow_always must not render a button via cross-layer path",
  );
  assert.ok(
    !html.includes("permission-decision-opt-reject-always"),
    "reject_always must not render a button via cross-layer path",
  );

  // Exactly two <button> elements.
  const buttonCount = (html.match(/<button/g) ?? []).length;
  assert.equal(
    buttonCount,
    2,
    `cross-layer four-option card must render exactly 2 buttons; got ${buttonCount}`,
  );
});

// ---------------------------------------------------------------------------
// Cross-layer reducer+mounted regression: channel_full leaves buttons disabled
//
// Proves that the full pipeline — acp_read → buildTranscript reducer →
// LifecycleActivity component — correctly leaves both buttons DISABLED after
// a `channel_full` control_result, matching the "transient, retransmit
// orchestrator keeps going" contract.
//
// The companion case proves authoritative failures (`no_active_turn`) DO
// re-enable buttons — so the effect path is also covered.
//
// Mutation proof: restoring `channel_full` to increment `deliveryFailed` in
// `handlePermissionDecisionResult` → the card acquires `deliveryFailed: 1` →
// the component re-renders with `deliveryFailed={1}` → the useEffect fires →
// `setPending(null)` re-enables both buttons → the disabled assertion fails.
// ---------------------------------------------------------------------------

test("test_channel_full_reducer_to_component_buttons_stay_disabled", async () => {
  const { createElement, act } = await import("react");
  const { render, fireEvent } = await import("@testing-library/react");

  const FAKE_NOW_SECS = Math.floor(FAKE_NOW_MS / 1000);
  const FUTURE_EXPIRY = FAKE_NOW_SECS + 9_999_999;
  const nonce = "nonce-cross-layer-cf";

  // Base acp_read event.
  const acpReadEvent = {
    seq: 1,
    timestamp: "2026-09-01T10:00:00.000Z",
    kind: "acp_read",
    agentIndex: 0,
    channelId: "ch-cross-cf",
    sessionId: "sess-cross-cf",
    turnId: "turn-cross-cf",
    payload: {
      jsonrpc: "2.0",
      id: "req-cross-cf",
      method: "session/request_permission",
      params: {
        title: "Tool requires approval",
        toolCallId: "tc-cross-cf",
        options: [
          {
            optionId: "opt-allow-once",
            kind: "allow_once",
            name: "Allow once",
          },
          { optionId: "opt-reject-once", kind: "reject_once", name: "Deny" },
        ],
      },
    },
    authorization: {
      requestNonce: nonce,
      actionable: true,
      expiresAt: FUTURE_EXPIRY,
    },
  };

  // `channel_full` control_result — transient; must NOT set deliveryFailed.
  const channelFullResult = {
    seq: 2,
    timestamp: "2026-09-01T10:00:01.000Z",
    kind: "control_result",
    agentIndex: 0,
    channelId: "ch-cross-cf",
    sessionId: "sess-cross-cf",
    turnId: "turn-cross-cf",
    payload: {
      type: "permission_decision",
      status: "channel_full",
      requestNonce: nonce,
      optionId: "opt-allow-once",
    },
  };

  // `no_active_turn` control_result — authoritative failure; MUST set deliveryFailed.
  const authoritativeFailure = {
    seq: 2,
    timestamp: "2026-09-01T10:00:01.000Z",
    kind: "control_result",
    agentIndex: 0,
    channelId: "ch-cross-cf",
    sessionId: "sess-cross-cf",
    turnId: "turn-cross-cf",
    payload: {
      type: "permission_decision",
      status: "no_active_turn",
      requestNonce: nonce,
      optionId: "opt-allow-once",
    },
  };

  // Build both card states through the real transcript reducer.
  const cardAfterChannelFull = buildTranscript([
    acpReadEvent,
    channelFullResult,
  ]).find((i) => i.renderClass === "permission");
  const cardAfterAuthoritativeFailure = buildTranscript([
    acpReadEvent,
    authoritativeFailure,
  ]).find((i) => i.renderClass === "permission");
  assert.ok(cardAfterChannelFull, "card must exist after channel_full");
  assert.ok(
    cardAfterAuthoritativeFailure,
    "card must exist after authoritative failure",
  );

  // Reducer-level gate: channel_full must NOT set deliveryFailed.
  assert.equal(
    cardAfterChannelFull.deliveryFailed,
    undefined,
    "channel_full must not set deliveryFailed in the reducer (mutation: restoring increment → 1 here → test fails)",
  );
  // Reducer-level gate: no_active_turn MUST set deliveryFailed.
  assert.equal(
    cardAfterAuthoritativeFailure.deliveryFailed,
    1,
    "no_active_turn must set deliveryFailed in the reducer",
  );

  // ── Component: channel_full → buttons stay disabled ───────────────────────
  // Start with the initial card (no deliveryFailed), click Allow to set pending.
  const initialCard = buildTranscript([acpReadEvent]).find(
    (i) => i.renderClass === "permission",
  );

  // Track delivery calls to ensure no second delivery is started.
  const deliveryCalls = [];
  // The first delivery is intentionally stalled — never resolves.
  function stalledDelivery({ optionId }) {
    deliveryCalls.push(optionId);
    return new Promise(() => {});
  }

  let container, rerender;
  await act(async () => {
    ({ container, rerender } = render(
      createElement(LifecycleActivity, {
        ...BASE_PROPS,
        item: initialCard,
        _deliveryFn: stalledDelivery,
      }),
    ));
  });

  // Click Allow — sets pending, disables both buttons.
  const allowBtn = container.querySelector(
    '[data-testid="permission-decision-opt-allow-once"]',
  );
  assert.ok(allowBtn, "allow_once button must be present before click");
  await act(async () => {
    fireEvent.click(allowBtn);
    await Promise.resolve();
  });
  assert.equal(deliveryCalls.length, 1, "first delivery must fire on click");

  // Now rerender with the post-channel_full card (deliveryFailed undefined).
  // The useEffect must NOT fire (deliveryFailed didn't change), so pending stays
  // set and both buttons remain disabled.
  await act(async () => {
    rerender(
      createElement(LifecycleActivity, {
        ...BASE_PROPS,
        item: cardAfterChannelFull,
        _deliveryFn: stalledDelivery,
      }),
    );
    await Promise.resolve();
  });

  const allowBtnAfterCF = container.querySelector(
    '[data-testid="permission-decision-opt-allow-once"]',
  );
  const denyBtnAfterCF = container.querySelector(
    '[data-testid="permission-decision-opt-reject-once"]',
  );
  assert.ok(allowBtnAfterCF, "allow button must still be in DOM");
  assert.ok(denyBtnAfterCF, "deny button must still be in DOM");
  assert.ok(
    allowBtnAfterCF.disabled,
    "allow button must remain DISABLED after channel_full (mutation: increment deliveryFailed → setPending(null) fires → button enabled → this fails)",
  );
  assert.ok(
    denyBtnAfterCF.disabled,
    "deny button must remain DISABLED after channel_full — both buttons stay disabled during automatic retry",
  );
  assert.equal(
    deliveryCalls.length,
    1,
    "no second delivery must start after channel_full — retransmit orchestrator handles resend, not a second click",
  );

  // ── Companion: authoritative failure re-enables buttons ───────────────────
  // Render a fresh card, click, then rerender with deliveryFailed: 1.
  const deliveryCalls2 = [];
  function stalledDelivery2({ optionId }) {
    deliveryCalls2.push(optionId);
    return new Promise(() => {});
  }

  let container2, rerender2;
  await act(async () => {
    ({ container: container2, rerender: rerender2 } = render(
      createElement(LifecycleActivity, {
        ...BASE_PROPS,
        item: initialCard,
        _deliveryFn: stalledDelivery2,
      }),
    ));
  });

  const allowBtn2 = container2.querySelector(
    '[data-testid="permission-decision-opt-allow-once"]',
  );
  assert.ok(allowBtn2, "allow button must be present for companion case");
  await act(async () => {
    fireEvent.click(allowBtn2);
    await Promise.resolve();
  });

  // Rerender with authoritative failure card (deliveryFailed: 1).
  // useEffect sees deliveryFailed change 0→1 → setPending(null) → buttons enabled.
  await act(async () => {
    rerender2(
      createElement(LifecycleActivity, {
        ...BASE_PROPS,
        item: cardAfterAuthoritativeFailure,
        _deliveryFn: stalledDelivery2,
      }),
    );
    await Promise.resolve();
    await Promise.resolve();
  });

  const allowBtnAfterFail = container2.querySelector(
    '[data-testid="permission-decision-opt-allow-once"]',
  );
  assert.ok(
    !allowBtnAfterFail.disabled,
    "allow button must be RE-ENABLED after no_active_turn — user can retry",
  );
});

// ---------------------------------------------------------------------------
// F3 interactive delivery-seam: acp_read → buildTranscript → LifecycleActivity
// click buttons → assert _deliveryFn called with ruled allow_once/reject_once IDs
//
// Mutation proof: removing `allow_once` from ACTIONABLE_KINDS → the allow_once
// button is not rendered → fireEvent.click finds no element → first delivery
// assertion fails. Removing `reject_once` → same for reject_once.
// Removing both → zero delivery calls → both assertions fail.
// ---------------------------------------------------------------------------

test("test_f3_interactive_delivery_seam_allow_once_and_reject_once_fire_delivery", async () => {
  const { createElement, act } = await import("react");
  const { render, fireEvent } = await import("@testing-library/react");

  const FAKE_NOW_SECS = Math.floor(FAKE_NOW_MS / 1000);
  const FUTURE_EXPIRY = FAKE_NOW_SECS + 9_999_999;

  // Record delivery calls: { optionId, requestNonce }[]
  const deliveryCalls = [];
  function mockDeliveryFn({ optionId, requestNonce }) {
    deliveryCalls.push({ optionId, requestNonce });
    // Resolve as "acked" so the component doesn't re-enable the button.
    return Promise.resolve("acked");
  }

  // Build the transcript card from a real acp_read event carrying all four
  // option kinds — same wire shape as the static cross-layer test above.
  const acpReadEvent = {
    seq: 1,
    timestamp: "2026-09-01T10:00:00.000Z",
    kind: "acp_read",
    agentIndex: 0,
    channelId: "ch-interactive",
    sessionId: "sess-interactive",
    turnId: "turn-interactive",
    payload: {
      jsonrpc: "2.0",
      id: "req-interactive",
      method: "session/request_permission",
      params: {
        title: "Tool requires approval",
        toolCallId: "tc-interactive",
        options: [
          {
            optionId: "opt-allow-once",
            kind: "allow_once",
            name: "Allow once",
          },
          { optionId: "opt-reject-once", kind: "reject_once", name: "Deny" },
          {
            optionId: "opt-allow-always",
            kind: "allow_always",
            name: "Always allow",
          },
          {
            optionId: "opt-reject-always",
            kind: "reject_always",
            name: "Always deny",
          },
        ],
      },
    },
    authorization: {
      requestNonce: "nonce-interactive",
      actionable: true,
      expiresAt: FUTURE_EXPIRY,
    },
  };

  const transcript = buildTranscript([acpReadEvent]);
  const card = transcript.find((item) => item.renderClass === "permission");
  assert.ok(card, "transcript must contain a permission card");
  assert.ok(card.actionable, "card must be actionable");

  let container;
  await act(async () => {
    ({ container } = render(
      createElement(LifecycleActivity, {
        ...BASE_PROPS,
        item: card,
        _deliveryFn: mockDeliveryFn,
      }),
    ));
  });

  // ── Click allow_once — must call delivery with opt-allow-once ─────────────
  const allowBtn = container.querySelector(
    '[data-testid="permission-decision-opt-allow-once"]',
  );
  assert.ok(
    allowBtn !== null,
    "allow_once button must be present (ACTIONABLE_KINDS must include allow_once)",
  );
  await act(async () => {
    fireEvent.click(allowBtn);
    // Drain microtasks so the async delivery fn resolves.
    await Promise.resolve();
    await Promise.resolve();
  });
  assert.equal(
    deliveryCalls.length,
    1,
    "exactly one delivery call after clicking allow_once; mutation: remove allow_once from ACTIONABLE_KINDS → zero calls",
  );
  assert.equal(
    deliveryCalls[0].optionId,
    "opt-allow-once",
    "delivery must be called with the allow_once optionId; mutation: wrong id → fails",
  );
  assert.equal(
    deliveryCalls[0].requestNonce,
    "nonce-interactive",
    "delivery must carry the card's requestNonce",
  );

  // ── allow_always must NOT have a button (not in ACTIONABLE_KINDS) ─────────
  assert.equal(
    container.querySelector(
      '[data-testid="permission-decision-opt-allow-always"]',
    ),
    null,
    "allow_always must not render a clickable button",
  );

  // ── reject_always must NOT have a button either ───────────────────────────
  assert.equal(
    container.querySelector(
      '[data-testid="permission-decision-opt-reject-always"]',
    ),
    null,
    "reject_always must not render a clickable button",
  );

  // ── Render a fresh card and click reject_once ──────────────────────────────
  // Use a separate render to avoid the pending-state from the allow_once click
  // disabling the reject_once button.
  deliveryCalls.length = 0;
  let container2;
  await act(async () => {
    ({ container: container2 } = render(
      createElement(LifecycleActivity, {
        ...BASE_PROPS,
        item: card,
        _deliveryFn: mockDeliveryFn,
      }),
    ));
  });

  const rejectBtn = container2.querySelector(
    '[data-testid="permission-decision-opt-reject-once"]',
  );
  assert.ok(
    rejectBtn !== null,
    "reject_once button must be present (ACTIONABLE_KINDS must include reject_once)",
  );
  await act(async () => {
    fireEvent.click(rejectBtn);
    await Promise.resolve();
    await Promise.resolve();
  });
  assert.equal(
    deliveryCalls.length,
    1,
    "exactly one delivery call after clicking reject_once; mutation: remove reject_once from ACTIONABLE_KINDS → zero calls",
  );
  assert.equal(
    deliveryCalls[0].optionId,
    "opt-reject-once",
    "delivery must be called with the reject_once optionId; mutation: wrong id → fails",
  );
});
