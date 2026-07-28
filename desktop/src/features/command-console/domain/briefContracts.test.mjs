import assert from "node:assert/strict";
import test from "node:test";

import {
  ADVISORY_LIMITATION,
  MAX_AGGREGATE_DISSENT_ITEMS,
  MAX_ARRAY_ITEMS,
  MAX_TEXT_BYTES,
  parseBriefRunState,
  parseBriefLifecycleRecord,
  parseBriefRunStatus,
  parseBriefSchedule,
  parseCommandBrief,
  parsePublishedCommandBrief,
} from "./briefContracts.ts";

const NOW = "2026-07-25T06:00:00Z";

function finding(text = "A supported finding.", sourceIds = ["ledger-1"]) {
  return { classification: "OFFICIAL", text, sourceIds };
}

function source(overrides = {}) {
  return {
    classification: "OFFICIAL",
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
    intelligence: "intelligence",
    logistics: "logistics",
    navigation: "navigation",
    daily_routine: "daily_routine",
    reporting: "reports",
    plans: "planning_30_60_90",
  }[adviser];
  return {
    classification: "OFFICIAL",
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
      intelligence: [],
      logistics: [],
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
    sourceFreshness: {
      classification: "OFFICIAL",
      asOf: NOW,
      staleSourceIds: [],
    },
    contributions: [
      contribution("operations"),
      contribution("intelligence"),
      contribution("logistics"),
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
    "intelligence",
    "logistics",
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
    brief({ sourceLedger: [source({ sourceKind: "unknown_osint" })] }),
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

test("requires explicit OFFICIAL classification on every nested wire object", () => {
  const missing = brief();
  delete missing.sourceLedger[0].classification;
  assert.equal(parseCommandBrief(missing), null);

  const publicSource = brief();
  publicSource.sourceLedger[0].classification = "PUBLIC";
  assert.equal(parseCommandBrief(publicSource), null);

  const mismatchedAction = brief();
  mismatchedAction.contributions[0].proposedActions = [
    {
      classification: "PUBLIC",
      actionId: "action-1",
      text: "This must remain a proposal.",
      approvalState: "pending",
    },
  ];
  assert.equal(parseCommandBrief(mismatchedAction), null);
});

test("requires final findings to exactly match specialist provenance", () => {
  const unsupported = brief();
  unsupported.sections.today[0].text = "A new claim with a valid citation.";
  assert.equal(parseCommandBrief(unsupported), null);

  assert.ok(parseCommandBrief(brief()));
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
    classification: "OFFICIAL",
    brief: brief(),
    lifecycleAuditEventId: "abcdef0123456789",
    publicationState: "queued",
  });
  assert.ok(published);
  assert.equal("lifecycleAuditEventId" in published.brief, false);
  assert.equal(Object.isFrozen(published), true);
});

test("accepts the seven-specialist aggregate dissent budget but no more", () => {
  const atLimitBrief = brief();
  const atLimit = [];
  atLimitBrief.contributions.forEach((specialist, specialistIndex) => {
    specialist.dissent = Array.from(
      { length: MAX_ARRAY_ITEMS },
      (_, index) => `dissent-${specialistIndex}-${index}`,
    );
    atLimit.push(...specialist.dissent);
  });
  atLimitBrief.dissent = atLimit;
  assert.equal(atLimit.length, MAX_AGGREGATE_DISSENT_ITEMS);
  assert.ok(parseCommandBrief(atLimitBrief));
  assert.equal(
    parseCommandBrief(brief({ dissent: [...atLimit, "one-too-many"] })),
    null,
  );

  const specialistOver = brief();
  specialistOver.contributions[0].dissent = Array.from(
    { length: MAX_ARRAY_ITEMS + 1 },
    (_, index) => `specialist-dissent-${index}`,
  );
  assert.equal(parseCommandBrief(specialistOver), null);
  assert.equal(
    parseCommandBrief(brief({ dissent: ["not preserved from a specialist"] })),
    null,
  );
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

test("requires OFFICIAL classification for schedule, run, and lifecycle records", () => {
  const schedule = {
    classification: "OFFICIAL",
    scheduleId: "daily-command-brief",
    enabled: true,
    localTime: "06:00",
    timezone: "Australia/Sydney",
    catchUpSameDay: true,
    concurrency: 1,
  };
  assert.ok(parseBriefSchedule(schedule));
  assert.equal(
    parseBriefSchedule({ ...schedule, classification: "PUBLIC" }),
    null,
  );

  const run = {
    classification: "OFFICIAL",
    runId: "run-1",
    scheduleId: "daily-command-brief",
    sequence: 0,
    state: "completed",
    updatedAt: NOW,
    degradedSections: [],
    error: null,
  };
  assert.ok(parseBriefRunStatus(run));
  const { classification: _, ...missingRunClassification } = run;
  assert.equal(parseBriefRunStatus(missingRunClassification), null);
  const { sequence: __, ...missingRunSequence } = run;
  assert.equal(parseBriefRunStatus(missingRunSequence), null);

  const lifecycle = {
    classification: "OFFICIAL",
    runId: "run-1",
    scheduleId: "daily-command-brief",
    state: "completed",
    occurredAt: NOW,
    snapshotId: "snapshot-1",
    previousLifecycleAuditEventId: null,
  };
  assert.ok(parseBriefLifecycleRecord(lifecycle));
  assert.equal(
    parseBriefLifecycleRecord({ ...lifecycle, classification: "PUBLIC" }),
    null,
  );
});
