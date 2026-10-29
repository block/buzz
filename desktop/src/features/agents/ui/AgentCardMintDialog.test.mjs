import assert from "node:assert/strict";
import { describe, it } from "node:test";

// Tests for the key-panel visibility derivations that AgentCardMintDialog
// imports from cardMintKeyUtils. These tests exercise the exact production
// module — changes to any exported function will cause failures here.

import {
  isReadOnlyLayer,
  isWritableLayer,
  keyPanelTitle,
  showCancelButton,
  showKeyPanel,
  showKeyStatusRow,
  showReadOnlyRow,
} from "./cardMintKeyUtils.ts";

describe("cardMintKeyUtils — key panel derivations", () => {
  // ── isWritableLayer ────────────────────────────────────────────────────────

  it("isWritableLayer_none_true", () => {
    assert.equal(isWritableLayer("none"), true);
  });

  it("isWritableLayer_global_true", () => {
    assert.equal(isWritableLayer("global"), true);
  });

  it("isWritableLayer_agent_false", () => {
    assert.equal(isWritableLayer("agent"), false);
  });

  it("isWritableLayer_persona_false", () => {
    assert.equal(isWritableLayer("persona"), false);
  });

  it("isWritableLayer_process_false", () => {
    assert.equal(isWritableLayer("process"), false);
  });

  it("isWritableLayer_undefined_false", () => {
    // Unknown (pending/error) — don't offer a write path
    assert.equal(isWritableLayer(undefined), false);
  });

  // ── isReadOnlyLayer ────────────────────────────────────────────────────────

  it("isReadOnlyLayer_agent_true", () => {
    assert.equal(isReadOnlyLayer("agent"), true);
  });

  it("isReadOnlyLayer_persona_true", () => {
    assert.equal(isReadOnlyLayer("persona"), true);
  });

  it("isReadOnlyLayer_process_true", () => {
    assert.equal(isReadOnlyLayer("process"), true);
  });

  it("isReadOnlyLayer_global_false", () => {
    assert.equal(isReadOnlyLayer("global"), false);
  });

  it("isReadOnlyLayer_none_false", () => {
    assert.equal(isReadOnlyLayer("none"), false);
  });

  it("isReadOnlyLayer_undefined_false", () => {
    assert.equal(isReadOnlyLayer(undefined), false);
  });

  // ── showKeyPanel ───────────────────────────────────────────────────────────
  // Key panel replaces the mint form ONLY for first-time setup (none) or
  // user-initiated editing (editingKey). Read-only layers do NOT replace the
  // mint form — they show an inline status row instead.

  it("showKeyPanel_none_notEditing_shows", () => {
    // First-time user: key not set → show setup panel
    assert.equal(showKeyPanel("none", false), true);
  });

  it("showKeyPanel_global_notEditing_hides", () => {
    // Normal state: key in global defaults, not editing → show mint form
    assert.equal(showKeyPanel("global", false), false);
  });

  it("showKeyPanel_global_editing_shows", () => {
    // User clicked Update → show the update panel
    assert.equal(showKeyPanel("global", true), true);
  });

  it("showKeyPanel_agent_notEditing_hides", () => {
    // Read-only layer: mint form stays visible; inline row shown instead
    assert.equal(showKeyPanel("agent", false), false);
  });

  it("showKeyPanel_persona_notEditing_hides", () => {
    assert.equal(showKeyPanel("persona", false), false);
  });

  it("showKeyPanel_process_notEditing_hides", () => {
    assert.equal(showKeyPanel("process", false), false);
  });

  it("showKeyPanel_agent_editing_shows", () => {
    // User clicked Why? on a read-only row → show the redirect panel
    assert.equal(showKeyPanel("agent", true), true);
  });

  it("showKeyPanel_persona_editing_shows", () => {
    assert.equal(showKeyPanel("persona", true), true);
  });

  it("showKeyPanel_process_editing_shows", () => {
    assert.equal(showKeyPanel("process", true), true);
  });

  it("showKeyPanel_undefined_notEditing_hides", () => {
    // Query pending/error → show mint form (fail-open, no panel claim)
    assert.equal(showKeyPanel(undefined, false), false);
  });

  // ── showCancelButton ───────────────────────────────────────────────────────

  it("showCancelButton_global_editing_shows", () => {
    // Update mode for a global key: Cancel returns to the mint form
    assert.equal(showCancelButton("global", true), true);
  });

  it("showCancelButton_none_editing_hides", () => {
    // First-time setup: no cancel (no mint form to return to)
    assert.equal(showCancelButton("none", true), false);
  });

  it("showCancelButton_global_notEditing_hides", () => {
    assert.equal(showCancelButton("global", false), false);
  });

  it("showCancelButton_agent_editing_shows", () => {
    // Read-only layer + user clicked Why?: Cancel returns to the mint form
    assert.equal(showCancelButton("agent", true), true);
  });

  it("showCancelButton_persona_editing_shows", () => {
    assert.equal(showCancelButton("persona", true), true);
  });

  it("showCancelButton_process_editing_shows", () => {
    assert.equal(showCancelButton("process", true), true);
  });

  // ── showKeyStatusRow ───────────────────────────────────────────────────────

  it("showKeyStatusRow_global_notEditing_shows", () => {
    // Confirmed writable key: show "Using your saved OpenAI key · Update"
    assert.equal(showKeyStatusRow("global", false), true);
  });

  it("showKeyStatusRow_global_editing_hides", () => {
    // In update panel: status row is redundant while editing
    assert.equal(showKeyStatusRow("global", true), false);
  });

  it("showKeyStatusRow_none_notEditing_hides", () => {
    // No key: show setup panel, not status row
    assert.equal(showKeyStatusRow("none", false), false);
  });

  it("showKeyStatusRow_agent_notEditing_hides", () => {
    // Read-only layer: use showReadOnlyRow instead
    assert.equal(showKeyStatusRow("agent", false), false);
  });

  it("showKeyStatusRow_undefined_notEditing_hides", () => {
    // Query pending/error: do not assert key existence
    assert.equal(showKeyStatusRow(undefined, false), false);
  });

  // ── showReadOnlyRow ────────────────────────────────────────────────────────
  // Inline provenance row on the mint form for keys the dialog cannot update.

  it("showReadOnlyRow_agent_notEditing_shows", () => {
    assert.equal(showReadOnlyRow("agent", false), true);
  });

  it("showReadOnlyRow_persona_notEditing_shows", () => {
    assert.equal(showReadOnlyRow("persona", false), true);
  });

  it("showReadOnlyRow_process_notEditing_shows", () => {
    assert.equal(showReadOnlyRow("process", false), true);
  });

  it("showReadOnlyRow_agent_editing_hides", () => {
    // User clicked Why? → redirect panel shown; row hidden
    assert.equal(showReadOnlyRow("agent", true), false);
  });

  it("showReadOnlyRow_global_notEditing_hides", () => {
    // Global key uses showKeyStatusRow instead
    assert.equal(showReadOnlyRow("global", false), false);
  });

  it("showReadOnlyRow_none_hides", () => {
    assert.equal(showReadOnlyRow("none", false), false);
  });

  it("showReadOnlyRow_undefined_hides", () => {
    assert.equal(showReadOnlyRow(undefined, false), false);
  });

  // ── Mint-reachability invariant ────────────────────────────────────────────
  // The mint form (and Mint button) must be reachable whenever a key resolves.
  // showKeyPanel returns true only for setup (none) or user-initiated edit.

  it("mintReachable_global_noEdit", () => {
    assert.equal(showKeyPanel("global", false), false);
  });

  it("mintReachable_agent_noEdit", () => {
    assert.equal(showKeyPanel("agent", false), false);
  });

  it("mintReachable_persona_noEdit", () => {
    assert.equal(showKeyPanel("persona", false), false);
  });

  it("mintReachable_process_noEdit", () => {
    assert.equal(showKeyPanel("process", false), false);
  });

  it("mintBlocked_none_noEdit", () => {
    // Only when no key is set at all does the panel gate minting
    assert.equal(showKeyPanel("none", false), true);
  });

  // ── keyPanelTitle ──────────────────────────────────────────────────────────

  it("keyPanelTitle_none_firstTimeSetup", () => {
    assert.equal(
      keyPanelTitle("none", false),
      "One-time setup: OpenAI API key",
    );
  });

  it("keyPanelTitle_global_editing_update", () => {
    assert.equal(keyPanelTitle("global", true), "Update OpenAI API key");
  });

  it("keyPanelTitle_agent_readOnly", () => {
    assert.equal(keyPanelTitle("agent", false), "OpenAI API key");
  });

  it("keyPanelTitle_persona_readOnly", () => {
    assert.equal(keyPanelTitle("persona", true), "OpenAI API key");
  });
});
