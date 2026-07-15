import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { communityRailIndicators } from "./CommunityRail.tsx";

describe("communityRailIndicators", () => {
  it("shows no badge for an observed community with unread but no mentions", () => {
    const r = communityRailIndicators({ hasUnread: true, state: "ready" });
    assert.equal(r.showBadge, false);
    assert.equal(r.showDot, true);
    assert.equal(r.pending, false);
  });

  it("shows no badge and no dot for an observed community with no unread", () => {
    const r = communityRailIndicators({ hasUnread: false, state: "ready" });
    assert.equal(r.showBadge, false);
    assert.equal(r.showDot, false);
    assert.equal(r.pending, false);
  });

  it("shows a mention badge with the count when mentions are present — no dot", () => {
    const r = communityRailIndicators({
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
    const r = communityRailIndicators({
      hasUnread: true,
      count: 250,
      state: "ready",
    });
    assert.equal(r.badgeLabel, "99+");
  });

  it("never reports mentions or dot for an unobserved (unknown) community", () => {
    const r = communityRailIndicators({
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
    const r = communityRailIndicators({ hasUnread: false, state: "loading" });
    assert.equal(r.pending, true);
    assert.equal(r.showBadge, false);
    assert.equal(r.showDot, false);
  });

  it("never reports mentions or dot on an errored observation", () => {
    const r = communityRailIndicators({
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
