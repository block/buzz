import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  workspaceInitials,
  workspaceRailIndicators,
} from "./WorkspaceRail.tsx";

describe("workspaceInitials", () => {
  it("filters punctuation before deriving initials", () => {
    assert.equal(workspaceInitials("B (relay)"), "BR");
  });
  it("handles a leading symbol on a single word", () => {
    assert.equal(workspaceInitials("(staging)"), "S");
  });
  it("still returns plain initials for normal names", () => {
    assert.equal(workspaceInitials("Bravo Beta"), "BB");
  });
  it("returns empty for a symbol-only name (caller falls back)", () => {
    assert.equal(workspaceInitials("()"), "");
  });
});

describe("workspaceRailIndicators", () => {
  it("shows no badge for an observed workspace with unread but no mentions", () => {
    const r = workspaceRailIndicators({ hasUnread: true, state: "ready" });
    assert.equal(r.showBadge, false);
    assert.equal(r.showDot, true);
    assert.equal(r.pending, false);
  });

  it("shows no badge and no dot for an observed workspace with no unread", () => {
    const r = workspaceRailIndicators({ hasUnread: false, state: "ready" });
    assert.equal(r.showBadge, false);
    assert.equal(r.showDot, false);
    assert.equal(r.pending, false);
  });

  it("shows a mention badge with the count when mentions are present — no dot", () => {
    const r = workspaceRailIndicators({
      hasUnread: true,
      count: 3,
      state: "ready",
    });
    assert.equal(r.showBadge, true);
    assert.equal(r.showDot, false);
    assert.equal(r.mentionCount, 3);
    assert.equal(r.badgeLabel, "3");
  });

  it("caps the badge label at 99+", () => {
    const r = workspaceRailIndicators({
      hasUnread: true,
      count: 250,
      state: "ready",
    });
    assert.equal(r.badgeLabel, "99+");
  });

  it("never reports mentions or dot for an unobserved (unknown) workspace", () => {
    const r = workspaceRailIndicators({
      hasUnread: true,
      count: 5,
      state: "unknown",
    });
    assert.equal(r.showBadge, false);
    assert.equal(r.showDot, false);
    assert.equal(r.mentionCount, 0);
    assert.equal(r.pending, true);
  });

  it("treats loading as pending — no badge, no dot", () => {
    const r = workspaceRailIndicators({ hasUnread: false, state: "loading" });
    assert.equal(r.pending, true);
    assert.equal(r.showBadge, false);
    assert.equal(r.showDot, false);
  });

  it("never reports mentions or dot on an errored observation", () => {
    const r = workspaceRailIndicators({
      hasUnread: true,
      count: 2,
      state: "error",
    });
    assert.equal(r.showBadge, false);
    assert.equal(r.showDot, false);
    assert.equal(r.mentionCount, 0);
    assert.equal(r.pending, false);
  });
});
