import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { DailyCommandBrief } from "./DailyCommandBrief.tsx";

const finding = {
  classification: "OFFICIAL",
  text: "Review the verified priority.",
  sourceIds: ["ledger-1"],
};
const advisers = [
  ["operations", "operations"],
  ["intelligence", "intelligence"],
  ["logistics", "logistics"],
  ["navigation", "navigation"],
  ["daily_routine", "daily_routine"],
  ["reporting", "reports"],
  ["plans", "planning_30_60_90"],
].map(([adviser, section], index) => ({
  classification: "OFFICIAL",
  adviser,
  section,
  findings: [finding],
  confidence: 0.8 - index * 0.05,
  limitations:
    adviser === "navigation" ? ["Chart update age requires review."] : [],
  dissent: adviser === "plans" ? ["Plans retains a dissenting view."] : [],
  proposedActions: [
    {
      classification: "OFFICIAL",
      actionId: `action-${index}`,
      text: "Prepare a workspace checklist proposal.",
      approvalState: "pending",
    },
  ],
}));

const sectionKeys = [
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
];
const sections = Object.fromEntries(sectionKeys.map((key) => [key, [finding]]));
const published = {
  classification: "OFFICIAL",
  lifecycleAuditEventId: "event-1",
  publicationState: "queued",
  brief: {
    version: 1,
    classification: "OFFICIAL",
    generatedAt: "2026-07-25T06:00:00Z",
    runId: "run-1",
    scheduleId: "daily-command-brief",
    snapshotId: "snapshot-verified",
    sections,
    degradedSections: ["navigation"],
    missingInformation: ["Reminders permission is denied."],
    dissent: ["Plans retains a dissenting view."],
    sourceLedger: [
      {
        classification: "OFFICIAL",
        ledgerId: "ledger-1",
        sourceId: "source-1",
        sourceKind: "rag",
        collection: "navigation",
        documentId: "doc-1",
        chunkId: "chunk-4",
        timestamp: "2026-07-25T05:30:00Z",
        snapshotId: "snapshot-verified",
        quotedLocation: { quote: "hidden evidence", location: "page 7" },
        retrievedAt: "2026-07-25T05:55:00Z",
        observedAt: "2026-07-25T05:56:00Z",
      },
      {
        classification: "OFFICIAL",
        ledgerId: "ledger-world-monitor",
        sourceId: "world-monitor-source",
        sourceKind: "world_monitor",
        collection: "World Monitor",
        documentId: "World Monitor regional update",
        chunkId: "world-monitor-chunk",
        timestamp: "2026-07-25T05:40:00Z",
        snapshotId: "snapshot-verified",
        quotedLocation: {
          quote: "hidden curated intelligence",
          location: "curated regional update",
        },
        retrievedAt: "2026-07-25T05:57:00Z",
        observedAt: "2026-07-25T05:58:00Z",
      },
    ],
    sourceFreshness: {
      classification: "OFFICIAL",
      asOf: "2026-07-25T05:56:00Z",
      staleSourceIds: ["ledger-1"],
    },
    contributions: advisers,
    advisoryLimitation:
      "This Daily Command Brief is advisory only. Navigation content identifies considerations and source limitations; it does not generate executable navigation orders or make navigational decisions.",
  },
};

const schedule = {
  classification: "OFFICIAL",
  scheduleId: "daily-command-brief",
  enabled: true,
  localTime: "06:00",
  timezone: "Australia/Sydney",
  catchUpSameDay: true,
  concurrency: 1,
};

const systemStatus = {
  degradedSections: ["navigation"],
  liveServices: [
    {
      id: "relay",
      label: "Command workspace",
      state: "connected",
      statusLabel: "Connected",
      detail: "Authenticated workspace connection is active.",
    },
    {
      id: "local-compute",
      label: "Local compute",
      state: "offline",
      statusLabel: "Offline",
      detail: "Local compute is not running.",
    },
    {
      id: "lm-studio",
      label: "LM Studio",
      state: "connected",
      statusLabel: "Connected",
      detail: "Local model is ready.",
    },
    {
      id: "memory",
      label: "Memory",
      state: "connected",
      statusLabel: "Connected",
      detail: "Memory is available.",
    },
    {
      id: "rag",
      label: "RAG",
      state: "connected",
      statusLabel: "Connected",
      detail: "RAG is available.",
    },
    {
      id: "apple-inputs",
      label: "Apple inputs",
      state: "degraded",
      statusLabel: "Degraded",
      detail: "Reminders permission is denied.",
    },
  ],
};

function render(overrides = {}) {
  return renderToStaticMarkup(
    React.createElement(DailyCommandBrief, {
      status: null,
      history: [],
      latest: null,
      schedule,
      loading: false,
      busy: false,
      error: null,
      onGenerate: () => {},
      onCancel: () => {},
      onScheduleChange: () => {},
      systemStatus,
      ...overrides,
    }),
  );
}

test("renders no-brief and queued/running/failed lifecycle states with truthful controls", () => {
  const initial = render();
  assert.match(initial, /No Daily Command Brief has been generated/);
  assert.match(
    initial,
    /latest available RAG, Memory, World Monitor, Calendar, Reminders, Notes/i,
  );
  assert.doesNotMatch(initial, /frozen OFFICIAL knowledge snapshot/i);

  const running = render({
    status: {
      classification: "OFFICIAL",
      runId: "run-1",
      scheduleId: "daily-command-brief",
      sequence: 1,
      state: "running_specialists",
      updatedAt: "2026-07-25T06:00:00Z",
      degradedSections: [],
      error: null,
    },
  });
  assert.match(running, />Running specialists</);
  assert.match(running, />Cancel generation</);
  assert.match(running, /aria-live="polite"/);

  const failed = render({
    status: {
      classification: "OFFICIAL",
      runId: "run-1",
      scheduleId: "daily-command-brief",
      sequence: 2,
      state: "failed",
      updatedAt: "2026-07-25T06:00:00Z",
      degradedSections: ["navigation"],
      error: "source_unavailable",
    },
  });
  assert.match(failed, />Failed</);
  assert.match(failed, /source_unavailable/);
});

test("renders the bounded native lifecycle history as metadata only", () => {
  const html = render({
    status: {
      classification: "OFFICIAL",
      runId: "run-1",
      scheduleId: "daily-command-brief",
      sequence: 1,
      state: "running_specialists",
      updatedAt: "2026-07-25T06:02:00Z",
      degradedSections: [],
      error: null,
    },
    history: [
      {
        classification: "OFFICIAL",
        runId: "run-1",
        scheduleId: "daily-command-brief",
        sequence: 0,
        state: "queued",
        updatedAt: "2026-07-25T06:00:00Z",
        degradedSections: [],
        error: null,
      },
      {
        classification: "OFFICIAL",
        runId: "run-1",
        scheduleId: "daily-command-brief",
        sequence: 1,
        state: "running_specialists",
        updatedAt: "2026-07-25T06:02:00Z",
        degradedSections: [],
        error: null,
      },
    ],
  });
  assert.match(html, />Lifecycle history</);
  assert.match(html, />Queued</);
  assert.match(html, />Running specialists</);
  assert.doesNotMatch(html, /prompt|reasoning|provider body/i);
});

test("renders a decision-first brief with supporting evidence collapsed after command content", () => {
  const html = render({
    latest: published,
    status: {
      classification: "OFFICIAL",
      runId: "run-1",
      scheduleId: "daily-command-brief",
      sequence: 7,
      state: "degraded",
      updatedAt: "2026-07-25T06:10:00Z",
      degradedSections: ["navigation"],
      error: null,
    },
  });

  const decisions = html.indexOf('data-testid="brief-section-decisions"');
  const today = html.indexOf('data-testid="brief-section-today"');
  const operations = html.indexOf('data-testid="brief-section-operations"');
  const intelligence = html.indexOf('data-testid="brief-section-intelligence"');
  const logistics = html.indexOf('data-testid="brief-section-logistics"');
  const navigation = html.indexOf('data-testid="brief-section-navigation"');
  assert.ok(
    decisions >= 0 &&
      decisions < today &&
      today < operations &&
      operations < intelligence &&
      intelligence < logistics &&
      logistics < navigation,
  );
  assert.doesNotMatch(html, />Generation status</);

  const disclosure = html.indexOf('data-testid="brief-evidence-disclosure"');
  assert.ok(disclosure > operations);
  assert.match(html, /<details[^>]*data-testid="brief-evidence-disclosure"/);
  assert.doesNotMatch(
    html.slice(0, disclosure),
    />Sources<|>Source ledger<|>Lifecycle history</,
  );
  assert.match(html, /Evidence and system status/);
  assert.match(html, /Specialist adviser contributions/);
  assert.match(html, /Source ledger/);
  assert.match(html, /System status/);

  for (const adviser of [
    "Operations",
    "Maritime N2",
    "Logistics",
    "Navigation",
    "Daily Routine",
    "Reporting",
    "Plans",
  ]) {
    assert.match(html, new RegExp(`>${adviser}<`));
  }
  assert.match(html, /80% confidence/);
  assert.match(html, /Chart update age requires review/);
  assert.match(html, /Plans retains a dissenting view/);
  assert.match(html, />Pending proposal</);
  assert.match(html, /href="#command-brief-source-ledger-1"/);
  assert.match(html, /Relay publication queued — offline capable/);
  assert.match(html, /Reminders permission is denied/);
  assert.match(html, /snapshot-verified/);
  assert.match(html, /Stale source/);
  assert.match(html, />Watch items</);
  assert.match(html, />Conflicts and gaps</);
  assert.doesNotMatch(html, /hidden evidence/);
  const worldMonitor = html.indexOf("World Monitor regional update");
  assert.ok(worldMonitor > disclosure);
  assert.doesNotMatch(
    html.slice(0, disclosure),
    /World Monitor regional update/,
  );
  assert.doesNotMatch(html, /approve|execute action/i);
});

test("restart view derives prominent degraded status and exact section labels from the immutable brief", () => {
  const html = render({ latest: published, status: null, history: [] });

  assert.match(html, />Complete with limitations</);
  assert.match(html, />Watch items</);
  assert.match(html, />Complete with limitations</);
  assert.match(html, />Navigation considerations</);
});

test("renders accessible schedule enable, time, and capacity controls with timezone context", () => {
  const html = render();
  assert.match(html, /aria-label="Enable scheduled Daily Command Brief"/);
  assert.match(html, /aria-label="Daily Command Brief local time"/);
  assert.match(html, /aria-label="Local model concurrency"/);
  assert.match(html, /Australia\/Sydney/);
  assert.match(html, />1 adviser at a time</);
  assert.match(html, />2 advisers at a time</);
});
