/**
 * Unit tests for the canOpenDraft openability predicate in DraftsPanel.
 *
 * These tests import and exercise the ACTUAL exported `canOpenDraft` function —
 * not a restatement of its logic — so any regression (e.g. reverting isSent or
 * source.channel checks, or removing the export) breaks these tests immediately.
 *
 * Three properties under test:
 *   (a) active draft + resolved channel          → canOpen = true
 *   (b) sent draft + resolved channel            → canOpen = false  (Delete-only)
 *   (c) active draft + unresolved channel (null) → canOpen = false  (false affordance guard)
 *   (d) active draft + empty channelId           → canOpen = false  (belt-and-suspenders)
 */

import assert from "node:assert/strict";
import test from "node:test";

// canOpenDraft is a pure function — no browser globals or React needed.
import { canOpenDraft } from "./DraftsPanel.tsx";

// Minimal Channel stub — only the fields canOpenDraft reads (none; it checks null/non-null).
const RESOLVED_CHANNEL = {
  id: "chan-1",
  visibility: "public",
  channelType: "channel",
};

function activeDraft(channelId = "chan-1") {
  return {
    channelId,
    content: "hello",
    createdAt: new Date().toISOString(),
    updatedAt: new Date().toISOString(),
    pendingImeta: [],
    status: "active",
  };
}

function sentDraft(channelId = "chan-1") {
  return { ...activeDraft(channelId), status: "sent" };
}

// ── (a) active + resolved channel → openable ──────────────────────────────────

test("canOpenDraft_active_resolved_channel_returns_true", () => {
  const draft = activeDraft("chan-1");
  const source = { channel: RESOLVED_CHANNEL, label: "#general" };
  assert.equal(
    canOpenDraft(draft, source),
    true,
    "active draft with resolved channel should be openable",
  );
});

// ── (b) sent + resolved channel → NOT openable ───────────────────────────────
// Composer restores only active/thread keys; sent: keys cannot be restored.
// Sent subsection is Delete-only (Will-confirmed behavior).

test("canOpenDraft_sent_resolved_channel_returns_false", () => {
  const draft = sentDraft("chan-1");
  const source = { channel: RESOLVED_CHANNEL, label: "#general" };
  assert.equal(
    canOpenDraft(draft, source),
    false,
    "sent draft should not be openable regardless of channel resolution",
  );
});

// ── (c) active + null channel → NOT openable ─────────────────────────────────
// Channel left/archived/unknown: routing to an empty channel surface is a false affordance.

test("canOpenDraft_active_null_channel_returns_false", () => {
  const draft = activeDraft("chan-gone");
  const source = { channel: null, label: "Unknown channel" };
  assert.equal(
    canOpenDraft(draft, source),
    false,
    "active draft with unresolved channel (null) should not be openable",
  );
});

// ── (d) active + empty channelId → NOT openable ──────────────────────────────
// Belt-and-suspenders: a draft with no channelId at all cannot be navigated to.

test("canOpenDraft_empty_channelId_returns_false", () => {
  const draft = activeDraft("");
  // channel stub present but channelId is empty — navigation would fail
  const source = { channel: RESOLVED_CHANNEL, label: "#general" };
  assert.equal(
    canOpenDraft(draft, source),
    false,
    "draft with empty channelId should not be openable",
  );
});

// ── (e) sent + null channel → NOT openable (doubly guarded) ──────────────────

test("canOpenDraft_sent_null_channel_returns_false", () => {
  const draft = sentDraft("chan-gone");
  const source = { channel: null, label: "Unknown channel" };
  assert.equal(
    canOpenDraft(draft, source),
    false,
    "sent draft with unresolved channel should not be openable",
  );
});
