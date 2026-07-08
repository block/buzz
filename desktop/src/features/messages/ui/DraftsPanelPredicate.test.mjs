/**
 * Unit tests for the canOpenDraft and canSendDraft predicates in DraftsPanel.
 *
 * These tests import and exercise the ACTUAL exported functions —
 * not a restatement of logic — so any regression breaks these tests immediately.
 *
 * canOpenDraft properties under test:
 *   (a) active draft + resolved channel          → canOpen = true
 *   (b) sent draft + resolved channel            → canOpen = false  (Delete-only)
 *   (c) active draft + unresolved channel (null) → canOpen = false  (false affordance guard)
 *   (d) active draft + empty channelId           → canOpen = false  (belt-and-suspenders)
 *
 * canSendDraft additional gates (mirrors ChannelPane.isComposerDisabled):
 *   (e) not a member     → canSend = false
 *   (f) archived channel → canSend = false
 *   (g) forum channel    → canSend = false
 *   (h) deleted root     → canSend = false
 *   (i) all-clear active + member + non-archived + non-forum → canSend = true
 */

import assert from "node:assert/strict";
import test from "node:test";

// canOpenDraft and canSendDraft are pure functions — no browser globals or React needed.
import { canOpenDraft, canSendDraft } from "./DraftsPanel.tsx";

// Minimal Channel stub — all fields canOpenDraft and canSendDraft read.
const RESOLVED_CHANNEL = {
  id: "chan-1",
  visibility: "public",
  channelType: "stream",
  isMember: true,
  archivedAt: null,
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

// ── canSendDraft: happy path ──────────────────────────────────────────────────

test("canSendDraft_active_member_stream_channel_returns_true", () => {
  const draft = activeDraft("chan-1");
  const source = { channel: RESOLVED_CHANNEL, label: "#general" };
  assert.equal(canSendDraft(draft, source, "available"), true);
});

// ── canSendDraft: deleted root ────────────────────────────────────────────────

test("canSendDraft_deleted_root_returns_false", () => {
  const draft = activeDraft("chan-1");
  const source = { channel: RESOLVED_CHANNEL, label: "#general" };
  assert.equal(
    canSendDraft(draft, source, "deleted"),
    false,
    "draft with deleted thread root must not be sendable",
  );
});

// ── canSendDraft: not a member ────────────────────────────────────────────────

test("canSendDraft_not_member_returns_false", () => {
  const draft = activeDraft("chan-1");
  const source = {
    channel: { ...RESOLVED_CHANNEL, isMember: false },
    label: "#general",
  };
  assert.equal(
    canSendDraft(draft, source, "available"),
    false,
    "draft in a channel the user is not a member of must not be sendable",
  );
});

// ── canSendDraft: archived channel ────────────────────────────────────────────

test("canSendDraft_archived_channel_returns_false", () => {
  const draft = activeDraft("chan-1");
  const source = {
    channel: { ...RESOLVED_CHANNEL, archivedAt: "2026-01-01T00:00:00Z" },
    label: "#archived",
  };
  assert.equal(
    canSendDraft(draft, source, "available"),
    false,
    "draft in an archived channel must not be sendable",
  );
});

// ── canSendDraft: forum channel ───────────────────────────────────────────────

test("canSendDraft_forum_channel_returns_false", () => {
  const draft = activeDraft("chan-1");
  const source = {
    channel: { ...RESOLVED_CHANNEL, channelType: "forum" },
    label: "#forum",
  };
  assert.equal(
    canSendDraft(draft, source, "available"),
    false,
    "draft in a forum channel must not be sendable (forum posting not wired)",
  );
});

// ── canSendDraft: checking/error roots are sendable ──────────────────────────

test("canSendDraft_checking_root_returns_true", () => {
  const draft = activeDraft("chan-1");
  const source = { channel: RESOLVED_CHANNEL, label: "#general" };
  assert.equal(
    canSendDraft(draft, source, "checking"),
    true,
    "checking root is optimistic — draft should still be sendable",
  );
});

test("canSendDraft_error_root_returns_true", () => {
  const draft = activeDraft("chan-1");
  const source = { channel: RESOLVED_CHANNEL, label: "#general" };
  assert.equal(
    canSendDraft(draft, source, "error"),
    true,
    "transport error root is optimistic — draft should still be sendable",
  );
});

// ── canSendDraft: sent draft ──────────────────────────────────────────────────

test("canSendDraft_sent_draft_returns_false", () => {
  const draft = sentDraft("chan-1");
  const source = { channel: RESOLVED_CHANNEL, label: "#general" };
  assert.equal(
    canSendDraft(draft, source, "available"),
    false,
    "sent draft is not sendable regardless of channel state",
  );
});
