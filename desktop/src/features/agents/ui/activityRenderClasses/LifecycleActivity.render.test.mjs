import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { LifecycleActivity } from "./LifecycleActivity.tsx";

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
// allow_once — renders a green actionable Allow button
// ---------------------------------------------------------------------------

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
// allow_always — non-actionable badge, no clickable button
// ---------------------------------------------------------------------------

test("test_allow_always_renders_non_actionable_persistent_grant_badge", () => {
  const html = renderToStaticMarkup(
    React.createElement(LifecycleActivity, {
      ...BASE_PROPS,
      item: pendingPermissionItem([
        { optionId: "opt-always", kind: "allow_always", label: "Always allow" },
      ]),
    }),
  );

  // Must show the non-actionable badge.
  assert.ok(
    html.includes("permission-decision-persistent-grant"),
    "allow_always option should render the persistent-grant badge",
  );
  assert.ok(
    html.includes("Permanent grant"),
    "persistent-grant badge should contain differentiating copy",
  );

  // Must NOT render a clickable button for this optionId.
  assert.ok(
    !html.includes("permission-decision-opt-always"),
    "allow_always option must not render an actionable button",
  );
  // No <button> element inside the buttons container at all.
  assert.ok(
    !html.includes("<button"),
    "allow_always-only card must not render any button element",
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
// reject_always with no label — actionable Deny labelled "Always deny"
// ---------------------------------------------------------------------------

test("test_reject_always_without_label_renders_always_deny_button", () => {
  const html = renderToStaticMarkup(
    React.createElement(LifecycleActivity, {
      ...BASE_PROPS,
      item: pendingPermissionItem([
        { optionId: "opt-reject-always", kind: "reject_always" },
      ]),
    }),
  );

  // Persistent denial stays actionable (fail-safe), but the harness omitted a
  // label — the default must disclose persistence, not read generic "Deny".
  assert.ok(
    html.includes("permission-decision-opt-reject-always"),
    "reject_always should render an actionable button",
  );
  assert.ok(
    html.includes("Always deny"),
    "reject_always without a label must default to 'Always deny', not 'Deny'",
  );
});

// ---------------------------------------------------------------------------
// Mixed options — allow_once + allow_always in same card
// ---------------------------------------------------------------------------

test("test_mixed_allow_once_and_allow_always_allow_once_actionable_always_non_actionable", () => {
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
  // The non-actionable persistent-grant badge appears.
  assert.ok(
    html.includes("permission-decision-persistent-grant"),
    "mixed card should show the persistent-grant badge for allow_always",
  );
});
