import assert from "node:assert/strict";
import test from "node:test";

import {
  ADVISORY_LIMITATION,
  MAX_ARRAY_ITEMS,
  MAX_TEXT_BYTES,
  parseBriefRunState,
  parseCommandBrief,
  parsePublishedCommandBrief,
} from "./briefContracts.ts";

const NOW = "2026-07-25T06:00:00Z";

function finding(text = "A supported finding.", sourceIds = ["ledger-1"]) {
  return { text, sourceIds };
}

function source(overrides = {}) {
  return {
    ledgerId: "ledger-1",
    sourceId: "source-1",
    sourceKind: "rag",
    collection: "engineering-orders",
    documentId: "document-1",
    chunkId: "chunk-1",
    timestamp: NOW,
    snapshotId: "snapshot-1",
    quotedLocation: { quote: "A supported quote.", location: "section 1" },
    retrievedAt: NOW,
    observedAt: NOW,
    ...overrides,
  };
}

function contribution(adviser, overrides = {}) {
  const section = {
    operations: "operations",
    navigation: "navigation",
    daily_routine: "daily_routine",
    reporting: "reports",
    plans: "planning_30_60_90",
  }[adviser];
  return {
    adviser,
    section,
    findings: [finding()],
    confidence: 0.85,
    limitations: ["Bounded to the frozen snapshot."],
    dissent: [],
    proposedActions: [],
    ...overrides,
  };
}

function brief(overrides = {}) {
  return {
    version: 1,
    classification: "OFFICIAL",
    generatedAt: NOW,
    runId: "run-1",
    scheduleId: "daily-command-brief",
    snapshotId: "snapshot-1",
    sections: {
      today: [finding()],
      operations: [],
      navigation: [],
      daily_routine: [],
      reports: [],
      planning_30_60_90: [],
      decisions: [],
      conflicts_and_gaps: [],
      sources: [],
    },
    degradedSections: [],
    missingInformation: [],
    dissent: [],
    sourceLedger: [source()],
    sourceFreshness: { asOf: NOW, staleSourceIds: [] },
    contributions: [
      contribution("operations"),
      contribution("navigation"),
      contribution("daily_routine"),
      contribution("reporting"),
      contribution("plans"),
    ],
    advisoryLimitation: ADVISORY_LIMITATION,
    ...overrides,
  };
}

test("parses and freezes the exact OFFICIAL brief wire shape", () => {
  const parsed = parseCommandBrief(JSON.parse(JSON.stringify(brief())));
  assert.ok(parsed);
  assert.equal(parsed.classification, "OFFICIAL");
  assert.equal(Object.isFrozen(parsed), true);
  assert.equal(Object.isFrozen(parsed.sourceLedger), true);
  assert.deepEqual(Object.keys(parsed.sections), [
    "today",
    "operations",
    "navigation",
    "daily_routine",
    "reports",
    "planning_30_60_90",
    "decisions",
    "conflicts_and_gaps",
    "sources",
  ]);
});

test("rejects extra keys, prototype pollution, unknown closed values, and unsafe classification", () => {
  for (const value of [
    brief({ unknown: true }),
    brief({ classification: "PUBLIC" }),
    brief({ sourceLedger: [source({ sourceKind: "network" })] }),
    brief({ contributions: [contribution("unapproved")] }),
    brief({ sections: { ...brief().sections, unknown: [] } }),
  ]) {
    assert.equal(parseCommandBrief(value), null);
  }

  const polluted = JSON.parse(JSON.stringify(brief()));
  Object.defineProperty(polluted, "__proto__", { value: { polluted: true } });
  assert.equal(parseCommandBrief(polluted), null);
  assert.equal({}.polluted, undefined);
});

test("rejects bad timestamps, duplicate IDs, missing citations, mixed snapshots, and stale IDs outside the ledger", () => {
  for (const value of [
    brief({ generatedAt: "not-a-timestamp" }),
    brief({ sourceLedger: [source(), source()] }),
    brief({
      contributions: [
        contribution("operations", { findings: [finding("x", ["missing"])] }),
      ],
    }),
    brief({ sourceLedger: [source({ snapshotId: "snapshot-2" })] }),
    brief({ sourceFreshness: { asOf: NOW, staleSourceIds: ["missing"] } }),
  ]) {
    assert.equal(parseCommandBrief(value), null);
  }
});

test("requires exactly one contribution from every specialist, safe confidence, and pending-only proposals", () => {
  const missing = brief();
  missing.contributions.pop();
  assert.equal(parseCommandBrief(missing), null);

  const duplicate = brief();
  duplicate.contributions[1] = contribution("operations");
  assert.equal(parseCommandBrief(duplicate), null);

  const confidence = brief();
  confidence.contributions[0].confidence = -0.01;
  assert.equal(parseCommandBrief(confidence), null);

  const approved = brief();
  approved.contributions[0].proposedActions = [
    { actionId: "a-1", text: "Must stay pending.", approvalState: "approved" },
  ];
  assert.equal(parseCommandBrief(approved), null);
});

test("navigation cannot encode orders or decisions and every brief retains the advisory limitation", () => {
  const order = brief();
  order.contributions[1].orders = ["Turn port immediately"];
  assert.equal(parseCommandBrief(order), null);

  const decision = brief();
  decision.contributions[1].decisions = ["Proceed"];
  assert.equal(parseCommandBrief(decision), null);

  assert.equal(
    parseCommandBrief(brief({ advisoryLimitation: "Different words" })),
    null,
  );
});

test("enforces text and array budgets, rejects controls, and only accepts post-signing event IDs in the wrapper", () => {
  for (const length of [MAX_TEXT_BYTES, MAX_TEXT_BYTES + 1]) {
    assert.equal(
      parseCommandBrief(brief({ missingInformation: ["x".repeat(length)] })) !==
        null,
      length === MAX_TEXT_BYTES,
    );
  }
  for (const count of [MAX_ARRAY_ITEMS, MAX_ARRAY_ITEMS + 1]) {
    assert.equal(
      parseCommandBrief(
        brief({
          missingInformation: Array.from(
            { length: count },
            (_, index) => `missing-${index}`,
          ),
        }),
      ) !== null,
      count === MAX_ARRAY_ITEMS,
    );
  }
  assert.equal(
    parseCommandBrief(brief({ missingInformation: ["bad\u0000"] })),
    null,
  );

  const published = parsePublishedCommandBrief({
    brief: brief(),
    lifecycleAuditEventId: "abcdef0123456789",
    publicationState: "queued",
  });
  assert.ok(published);
  assert.equal("lifecycleAuditEventId" in published.brief, false);
  assert.equal(Object.isFrozen(published), true);
});

test("accepts only the closed Rust run-state vocabulary", () => {
  for (const state of [
    "queued",
    "collecting_sources",
    "running_specialists",
    "consolidating",
    "persisting",
    "completed",
    "degraded",
    "cancelled",
    "failed",
  ]) {
    assert.equal(parseBriefRunState(state), state);
  }
  assert.equal(parseBriefRunState("invented"), null);
});
