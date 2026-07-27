import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";

import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

const finding = (text: string) => ({
  classification: "OFFICIAL",
  sourceIds: ["ledger-1"],
  text,
});

const sectionFindings = {
  decisions: [finding("Confirm the command priorities for today's programme.")],
  today: [finding("Review the morning programme and critical timings.")],
  operations: [
    finding("Confirm readiness dependencies before the next activity."),
  ],
  navigation: [
    finding("Review the cited navigation considerations and limitations."),
  ],
  daily_routine: [
    finding("Calendar and routine inputs are assembled for review."),
  ],
  reports: [
    finding("Confirm owners for reports and returns approaching due date."),
  ],
  planning_30_60_90: [
    finding("Review near-term milestones, dependencies and decision points."),
  ],
  conflicts_and_gaps: [],
  sources: [finding("Source evidence is retained in the ledger.")],
};

const specialistFindings = {
  operations: [
    ...sectionFindings.decisions,
    ...sectionFindings.today,
    ...sectionFindings.operations,
    ...sectionFindings.sources,
  ],
  navigation: sectionFindings.navigation,
  daily_routine: sectionFindings.daily_routine,
  reports: sectionFindings.reports,
  planning_30_60_90: sectionFindings.planning_30_60_90,
};

const publishedBrief = {
  classification: "OFFICIAL",
  lifecycleAuditEventId: "event-naval-ui",
  publicationState: "published",
  brief: {
    version: 1,
    classification: "OFFICIAL",
    generatedAt: "2026-07-27T06:00:00+10:00",
    runId: "run-naval-ui",
    scheduleId: "daily-command-brief",
    snapshotId: "snapshot-naval-ui",
    sections: sectionFindings,
    degradedSections: [],
    missingInformation: [],
    dissent: [],
    sourceLedger: [
      {
        classification: "OFFICIAL",
        ledgerId: "ledger-1",
        sourceId: "source-1",
        sourceKind: "rag",
        collection: "command-knowledge",
        documentId: "document-1",
        chunkId: "chunk-1",
        timestamp: "2026-07-27T05:30:00+10:00",
        snapshotId: "snapshot-naval-ui",
        quotedLocation: {
          quote: "Evidence remains hidden from the primary command view.",
          location: "section 1",
        },
        retrievedAt: "2026-07-27T05:55:00+10:00",
        observedAt: "2026-07-27T05:56:00+10:00",
      },
    ],
    sourceFreshness: {
      classification: "OFFICIAL",
      asOf: "2026-07-27T05:56:00+10:00",
      staleSourceIds: [],
    },
    contributions: [
      ["operations", "operations"],
      ["navigation", "navigation"],
      ["daily_routine", "daily_routine"],
      ["reporting", "reports"],
      ["plans", "planning_30_60_90"],
    ].map(([adviser, section], index) => ({
      classification: "OFFICIAL",
      adviser,
      section,
      findings: specialistFindings[section as keyof typeof specialistFindings],
      confidence: 0.9 - index * 0.05,
      limitations: [],
      dissent: [],
      proposedActions: [],
    })),
    advisoryLimitation:
      "This Daily Command Brief is advisory only. Navigation content identifies considerations and source limitations; it does not generate executable navigation orders or make navigational decisions.",
  },
};

const completedStatus = {
  classification: "OFFICIAL",
  runId: "run-naval-ui",
  scheduleId: "daily-command-brief",
  sequence: 7,
  state: "completed",
  updatedAt: "2026-07-27T06:01:00+10:00",
  degradedSections: [],
  error: null,
};

async function sha256(path: string) {
  return createHash("sha256")
    .update(await readFile(path))
    .digest("hex");
}

test("matches the selected naval briefing direction", async ({ page }) => {
  await page.setViewportSize({ height: 720, width: 1280 });
  await installMockBridge(page, {
    commandBriefLatest: publishedBrief,
    commandBriefStatus: {
      classification: "OFFICIAL",
      current: completedStatus,
      history: [completedStatus],
    },
  });
  await page.goto("/");
  await page.getByTestId("open-command-console-view").click();

  const consoleScreen = page.getByTestId("command-console-screen");
  await expect(consoleScreen).toHaveCSS("background-color", "rgb(3, 20, 38)");
  await expect(
    consoleScreen.getByRole("img", { name: "HMAS Supply badge" }),
  ).toBeVisible();
  await expect(
    consoleScreen.getByRole("img", { name: "HMAS Supply at sea" }),
  ).toBeVisible();
  const team = consoleScreen.getByTestId("command-team");
  await expect(team).toBeVisible();
  for (const adviser of [
    "chief-of-staff",
    "operations",
    "navigation",
    "daily-routine",
    "reporting",
    "plans",
  ]) {
    await expect(team.getByTestId(`adviser-insignia-${adviser}`)).toBeVisible();
  }
  await expect(
    consoleScreen.getByRole("button", { name: "Cloud models first" }),
  ).toBeVisible();
  await expect(
    consoleScreen.getByTestId("brief-section-decisions"),
  ).toBeVisible();

  const defaultPath =
    "test-results/command-adviser-naval-ui/default-briefing.png";
  await consoleScreen.evaluate((element) => {
    element.scrollTop = 0;
  });
  await waitForAnimations(page);
  await page.screenshot({ path: defaultPath });

  const disclosure = consoleScreen.getByTestId("brief-evidence-disclosure");
  await disclosure.getByText("Evidence and system status").click();
  await expect(disclosure).toHaveAttribute("open", "");
  await expect(consoleScreen.getByText("Source ledger")).toBeVisible();
  await expect(
    consoleScreen.getByTestId("command-system-status"),
  ).toBeVisible();
  await disclosure.scrollIntoViewIfNeeded();

  const expandedPath =
    "test-results/command-adviser-naval-ui/expanded-evidence.png";
  await waitForAnimations(page);
  await page.screenshot({ path: expandedPath });

  expect(await sha256(defaultPath)).not.toBe(await sha256(expandedPath));
});
