/**
 * Named test matrix for the label-fix: describePermissionOutcome and
 * describePermissionTerminalReason must render the harness-provided label,
 * never the raw ACP kind string.
 *
 * Covers dispatch item 6 (2a label fix) from the Phase-2 brief.
 */
import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  describePermissionOutcome,
  describePermissionTerminalReason,
} from "./agentSessionTranscriptPermissions.ts";

// ── Fixtures ──────────────────────────────────────────────────────────────────

/** ACP kind → harness label mapping as would arrive from describePermissionRequest */
const ALLOW_ONCE_LABELS = new Map([["opt-allow-once", "Allow once"]]);
const ALLOW_ONCE_KINDS = new Map([["opt-allow-once", "allow_once"]]);

const ALLOW_ALWAYS_LABELS = new Map([["opt-allow-always", "Always allow"]]);
const ALLOW_ALWAYS_KINDS = new Map([["opt-allow-always", "allow_always"]]);

const DENY_LABELS = new Map([["opt-deny", "Deny"]]);
const DENY_KINDS = new Map([["opt-deny", "reject_once"]]);

const EMPTY = new Map();

// ── describePermissionOutcome ─────────────────────────────────────────────────

describe("describePermissionOutcome — label rendering", () => {
  it("test_label_fix_renders_harness_label_not_raw_kind", () => {
    // The core regression: must return "Allow once", not "Approved (allow_once)"
    const result = describePermissionOutcome(
      "selected",
      "opt-allow-once",
      ALLOW_ONCE_LABELS,
      ALLOW_ONCE_KINDS,
    );
    assert.equal(result, "Allow once");
    assert.ok(!result.includes("allow_once"), "must not contain raw ACP kind");
  });

  it("test_label_fix_deny_renders_harness_label_not_raw_kind", () => {
    const result = describePermissionOutcome(
      "selected",
      "opt-deny",
      DENY_LABELS,
      DENY_KINDS,
    );
    assert.equal(result, "Deny");
    assert.ok(!result.includes("reject_once"), "must not contain raw ACP kind");
  });

  it("test_label_fix_always_allow_renders_harness_label", () => {
    const result = describePermissionOutcome(
      "selected",
      "opt-allow-always",
      ALLOW_ALWAYS_LABELS,
      ALLOW_ALWAYS_KINDS,
    );
    assert.equal(result, "Always allow");
    assert.ok(
      !result.includes("allow_always"),
      "must not contain raw ACP kind",
    );
  });

  it("test_no_label_falls_back_to_verb_only_not_kind", () => {
    // When no harness label is available, render verb only ("Approved" / "Denied"),
    // never the raw kind string.
    const result = describePermissionOutcome(
      "selected",
      "opt-allow-once",
      EMPTY, // no labels
      ALLOW_ONCE_KINDS,
    );
    assert.equal(result, "Approved");
    assert.ok(!result.includes("allow_once"), "must not contain raw ACP kind");
  });

  it("test_no_label_deny_verb_fallback", () => {
    const result = describePermissionOutcome(
      "selected",
      "opt-deny",
      EMPTY,
      DENY_KINDS,
    );
    assert.equal(result, "Denied");
    assert.ok(!result.includes("reject_once"), "must not contain raw ACP kind");
  });

  it("test_cancelled_outcome", () => {
    assert.equal(
      describePermissionOutcome("cancelled", null, EMPTY),
      "Cancelled",
    );
  });

  it("test_timed_out_outcome", () => {
    assert.equal(
      describePermissionOutcome("timed_out", null, EMPTY),
      "Timed out",
    );
  });

  it("test_uncertain_outcome_verbatim", () => {
    const result = describePermissionOutcome("uncertain", null, EMPTY);
    assert.equal(
      result,
      "Approval outcome unknown; agent process stopped before it could continue.",
    );
  });

  it("test_unknown_outcome_passthrough", () => {
    // Unknown outcomes pass through unchanged.
    assert.equal(
      describePermissionOutcome("some_new_outcome", null, EMPTY),
      "some_new_outcome",
    );
  });
});

// ── describePermissionTerminalReason ─────────────────────────────────────────

describe("describePermissionTerminalReason — label rendering", () => {
  const OPTIONS_WITH_LABEL = [
    { optionId: "opt-allow-once", kind: "allow_once", label: "Allow once" },
    {
      optionId: "opt-allow-always",
      kind: "allow_always",
      label: "Always allow",
    },
    { optionId: "opt-deny", kind: "reject_once", label: "Deny" },
  ];

  const OPTIONS_WITHOUT_LABEL = [
    { optionId: "opt-allow-once", kind: "allow_once" },
    { optionId: "opt-deny", kind: "reject_once" },
  ];

  it("test_terminal_reason_applied_renders_harness_label", () => {
    const result = describePermissionTerminalReason(
      "applied",
      "selected",
      "opt-allow-once",
      OPTIONS_WITH_LABEL,
    );
    assert.equal(result, "Allow once");
    assert.ok(!result.includes("allow_once"), "must not contain raw ACP kind");
  });

  it("test_terminal_reason_applied_always_allow_label", () => {
    const result = describePermissionTerminalReason(
      "applied",
      "selected",
      "opt-allow-always",
      OPTIONS_WITH_LABEL,
    );
    assert.equal(result, "Always allow");
  });

  it("test_terminal_reason_applied_deny_renders_harness_label", () => {
    const result = describePermissionTerminalReason(
      "applied",
      "selected",
      "opt-deny",
      OPTIONS_WITH_LABEL,
    );
    assert.equal(result, "Deny");
    assert.ok(!result.includes("reject_once"), "must not contain raw ACP kind");
  });

  it("test_terminal_reason_applied_no_label_falls_back_to_verb", () => {
    // Options without a label field: verb-only fallback, never raw kind.
    const result = describePermissionTerminalReason(
      "applied",
      "selected",
      "opt-allow-once",
      OPTIONS_WITHOUT_LABEL,
    );
    assert.equal(result, "Approved");
    assert.ok(!result.includes("allow_once"), "must not contain raw ACP kind");
  });

  it("test_terminal_reason_timed_out", () => {
    assert.equal(
      describePermissionTerminalReason("timed_out", null, null, []),
      "Timed out",
    );
  });

  it("test_terminal_reason_cancelled", () => {
    assert.equal(
      describePermissionTerminalReason("cancelled", null, null, []),
      "Cancelled",
    );
  });

  it("test_terminal_reason_uncertain_verbatim", () => {
    assert.equal(
      describePermissionTerminalReason("uncertain", null, null, []),
      "Approval outcome unknown; agent process stopped before it could continue.",
    );
  });

  it("test_terminal_no_reason_falls_back_to_outcome", () => {
    // Reason absent → outcome-level fallback, still renders label not kind.
    const result = describePermissionTerminalReason(
      undefined,
      "selected",
      "opt-allow-once",
      OPTIONS_WITH_LABEL,
    );
    assert.equal(result, "Allow once");
  });

  it("test_terminal_no_reason_no_options_passes_through", () => {
    // No reason, no options, unknown outcome → passthrough.
    const result = describePermissionTerminalReason(
      undefined,
      "some_outcome",
      null,
      [],
    );
    assert.equal(result, "some_outcome");
  });
});
