import assert from "node:assert/strict";
import { test } from "node:test";

import {
  buildModerationQueue,
  groupTopReportType,
  isOpenReport,
  reportSeverity,
  reportTypeLabel,
  severityTier,
  targetKey,
} from "./moderationQueue.ts";

function report(overrides = {}) {
  return {
    id: overrides.id ?? "r1",
    reportEventId: overrides.reportEventId ?? "e".repeat(64),
    reporterPubkey: overrides.reporterPubkey ?? "a".repeat(64),
    targetKind: overrides.targetKind ?? "event",
    target: overrides.target ?? "t".repeat(64),
    channelId: overrides.channelId ?? null,
    reportType: overrides.reportType ?? "spam",
    note: overrides.note ?? null,
    status: overrides.status ?? "open",
    resolvedBy: overrides.resolvedBy ?? null,
    resolvedAt: overrides.resolvedAt ?? null,
    actionId: overrides.actionId ?? null,
    createdAt: overrides.createdAt ?? "2026-07-07T00:00:00.000Z",
  };
}

function action(overrides = {}) {
  return {
    id: overrides.id ?? "a1",
    actorPubkey: overrides.actorPubkey ?? "b".repeat(64),
    action: overrides.action ?? "timeout",
    targetPubkey: overrides.targetPubkey ?? null,
    targetEventId: overrides.targetEventId ?? null,
    channelId: overrides.channelId ?? null,
    reasonCode: overrides.reasonCode ?? null,
    publicReason: overrides.publicReason ?? null,
    privateReason: overrides.privateReason ?? null,
    matchedPrincipal: overrides.matchedPrincipal ?? null,
    createdAt: overrides.createdAt ?? "2026-07-06T00:00:00.000Z",
  };
}

test("reportSeverity: illegal outranks all; other is lowest", () => {
  assert.ok(reportSeverity("illegal") > reportSeverity("malware"));
  assert.ok(reportSeverity("malware") > reportSeverity("spam"));
  assert.ok(reportSeverity("spam") > reportSeverity("profanity"));
  assert.ok(reportSeverity("profanity") > reportSeverity("other"));
  assert.equal(reportSeverity("other"), 0);
});

test("targetKey is kind-qualified so event/pubkey with same hex don't collide", () => {
  const hex = "c".repeat(64);
  assert.notEqual(
    targetKey(report({ targetKind: "event", target: hex })),
    targetKey(report({ targetKind: "pubkey", target: hex })),
  );
});

test("buildModerationQueue collapses reports about the same target into one group", () => {
  const t = "d".repeat(64);
  const groups = buildModerationQueue([
    report({ id: "r1", target: t, reporterPubkey: "1".repeat(64) }),
    report({ id: "r2", target: t, reporterPubkey: "2".repeat(64) }),
  ]);
  assert.equal(groups.length, 1);
  assert.equal(groups[0].reports.length, 2);
});

test("group maxSeverity is the highest among its reports", () => {
  const t = "d".repeat(64);
  const [group] = buildModerationQueue([
    report({ id: "r1", target: t, reportType: "spam" }),
    report({ id: "r2", target: t, reportType: "illegal" }),
  ]);
  assert.equal(group.maxSeverity, reportSeverity("illegal"));
});

test("groups sort by severity desc, then most-recent report desc", () => {
  const groups = buildModerationQueue([
    report({
      id: "low",
      target: "1".repeat(64),
      reportType: "profanity",
      createdAt: "2026-07-07T09:00:00.000Z",
    }),
    report({
      id: "high",
      target: "2".repeat(64),
      reportType: "illegal",
      createdAt: "2026-07-07T01:00:00.000Z",
    }),
    report({
      id: "midNew",
      target: "3".repeat(64),
      reportType: "spam",
      createdAt: "2026-07-07T10:00:00.000Z",
    }),
    report({
      id: "midOld",
      target: "4".repeat(64),
      reportType: "spam",
      createdAt: "2026-07-07T02:00:00.000Z",
    }),
  ]);
  assert.deepEqual(
    groups.map((g) => g.reports[0].id),
    ["high", "midNew", "midOld", "low"],
  );
});

test("reports within a group are newest-first; latestCreatedAt reflects that", () => {
  const t = "d".repeat(64);
  const [group] = buildModerationQueue([
    report({ id: "old", target: t, createdAt: "2026-07-01T00:00:00.000Z" }),
    report({ id: "new", target: t, createdAt: "2026-07-05T00:00:00.000Z" }),
  ]);
  assert.equal(group.reports[0].id, "new");
  assert.equal(group.latestCreatedAt, "2026-07-05T00:00:00.000Z");
});

test("prior actions correlate to event-targeted groups via targetEventId", () => {
  const eventId = "e".repeat(64);
  const [group] = buildModerationQueue(
    [report({ targetKind: "event", target: eventId })],
    [
      action({
        id: "match",
        targetEventId: eventId,
        createdAt: "2026-07-02T00:00:00.000Z",
      }),
      action({
        id: "matchNewer",
        targetEventId: eventId,
        createdAt: "2026-07-04T00:00:00.000Z",
      }),
      action({ id: "other", targetEventId: "f".repeat(64) }),
    ],
  );
  assert.deepEqual(
    group.priorActions.map((a) => a.id),
    ["matchNewer", "match"],
  );
});

test("prior actions correlate to pubkey-targeted groups via targetPubkey", () => {
  const pk = "9".repeat(64);
  const [group] = buildModerationQueue(
    [report({ targetKind: "pubkey", target: pk })],
    [action({ id: "ban", action: "ban", targetPubkey: pk })],
  );
  assert.deepEqual(
    group.priorActions.map((a) => a.id),
    ["ban"],
  );
});

test("blob-targeted groups surface no prior-actions correlation (audit has no blob key)", () => {
  const sha = "7".repeat(64);
  const [group] = buildModerationQueue(
    [report({ targetKind: "blob", target: sha })],
    [action({ targetEventId: sha }), action({ targetPubkey: sha })],
  );
  assert.equal(group.priorActions.length, 0);
});

test("isOpenReport is true only for open status", () => {
  assert.equal(isOpenReport(report({ status: "open" })), true);
  assert.equal(isOpenReport(report({ status: "resolved" })), false);
  assert.equal(isOpenReport(report({ status: "escalated" })), false);
});

test("empty input yields empty queue", () => {
  assert.deepEqual(buildModerationQueue([]), []);
});

test("reportTypeLabel covers every category", () => {
  for (const t of [
    "illegal",
    "nudity",
    "malware",
    "spam",
    "impersonation",
    "profanity",
    "other",
  ]) {
    assert.equal(typeof reportTypeLabel(t), "string");
    assert.ok(reportTypeLabel(t).length > 0);
  }
});

test("severityTier: illegal=critical, malware/impersonation=high, rest=normal", () => {
  assert.equal(severityTier("illegal"), "critical");
  assert.equal(severityTier("malware"), "high");
  assert.equal(severityTier("impersonation"), "high");
  assert.equal(severityTier("spam"), "normal");
  assert.equal(severityTier("nudity"), "normal");
  assert.equal(severityTier("profanity"), "normal");
  assert.equal(severityTier("other"), "normal");
});

test("groupTopReportType returns the most severe type in a group", () => {
  const t = "d".repeat(64);
  const [group] = buildModerationQueue([
    report({ id: "r1", target: t, reportType: "spam" }),
    report({ id: "r2", target: t, reportType: "impersonation" }),
    report({ id: "r3", target: t, reportType: "profanity" }),
  ]);
  assert.equal(groupTopReportType(group), "impersonation");
});
