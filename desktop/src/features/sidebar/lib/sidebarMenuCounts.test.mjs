import assert from "node:assert/strict";
import test from "node:test";

import {
  deriveSidebarMenuCounts,
  formatSidebarAgentsCount,
  formatSidebarMenuCount,
  resolveSidebarMenuCount,
  shouldShowSidebarMenuCount,
} from "./sidebarMenuCounts.ts";

test("formatSidebarMenuCount caps at 99 and floors negatives to zero", () => {
  assert.equal(formatSidebarMenuCount(0), 0);
  assert.equal(formatSidebarMenuCount(12), 12);
  assert.equal(formatSidebarMenuCount(99), 99);
  assert.equal(formatSidebarMenuCount(100), 99);
  assert.equal(formatSidebarMenuCount(3.9), 3);
  assert.equal(formatSidebarMenuCount(-2), 0);
});

test("resolveSidebarMenuCount hides undefined loading values", () => {
  assert.equal(resolveSidebarMenuCount(undefined), undefined);
  assert.equal(resolveSidebarMenuCount(null), undefined);
  assert.equal(resolveSidebarMenuCount(Number.NaN), undefined);
  assert.equal(resolveSidebarMenuCount(-1), undefined);
  assert.equal(resolveSidebarMenuCount(0), 0);
  assert.equal(resolveSidebarMenuCount(42), 42);
});

test("formatSidebarAgentsCount is running/total", () => {
  assert.equal(formatSidebarAgentsCount(3, 12), "3/12");
  assert.equal(formatSidebarAgentsCount(0, 5), "0/5");
  assert.equal(formatSidebarAgentsCount(100, 200), "99/99");
  assert.equal(formatSidebarAgentsCount(undefined, 5), undefined);
  assert.equal(formatSidebarAgentsCount(2, undefined), undefined);
});

test("shouldShowSidebarMenuCount shows zeros only when preference is on", () => {
  assert.equal(
    shouldShowSidebarMenuCount({
      preferenceEnabled: true,
      count: 0,
    }),
    true,
  );
  assert.equal(
    shouldShowSidebarMenuCount({
      preferenceEnabled: false,
      count: 0,
      legacyWhenPositive: true,
    }),
    false,
  );
  assert.equal(
    shouldShowSidebarMenuCount({
      preferenceEnabled: false,
      count: 3,
      legacyWhenPositive: true,
    }),
    true,
  );
  assert.equal(
    shouldShowSidebarMenuCount({
      preferenceEnabled: false,
      count: 5,
    }),
    false,
  );
  assert.equal(
    shouldShowSidebarMenuCount({
      preferenceEnabled: true,
      count: undefined,
    }),
    false,
  );
  assert.equal(
    shouldShowSidebarMenuCount({
      preferenceEnabled: true,
      count: "3/12",
    }),
    true,
  );
  assert.equal(
    shouldShowSidebarMenuCount({
      preferenceEnabled: false,
      count: "3/12",
    }),
    false,
  );
});

test("deriveSidebarMenuCounts maps inbox unread, browser attention sum, agents X/Y", () => {
  assert.deepEqual(
    deriveSidebarMenuCounts({
      inboxUnread: 7,
      browserAttentionCount: 2,
      agentRunningCount: 3,
      agentTotalCount: 12,
      botCount: 11,
    }),
    { inbox: 7, browsers: 2, agents: "3/12", bots: 11 },
  );
  assert.deepEqual(
    deriveSidebarMenuCounts({
      inboxUnread: undefined,
      browserAttentionCount: 0,
      agentRunningCount: undefined,
      agentTotalCount: undefined,
      botCount: undefined,
    }),
    {
      inbox: undefined,
      browsers: 0,
      agents: undefined,
      bots: undefined,
    },
  );
  // Attention = new browsers + pending runbooks (caller sums).
  assert.deepEqual(
    deriveSidebarMenuCounts({
      inboxUnread: 0,
      browserAttentionCount: 1 + 4,
      agentRunningCount: 0,
      agentTotalCount: 2,
      botCount: 0,
    }),
    { inbox: 0, browsers: 5, agents: "0/2", bots: 0 },
  );
});
