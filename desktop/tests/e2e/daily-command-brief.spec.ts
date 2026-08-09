import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const ADVISORY_LIMITATION =
  "This Daily Command Brief is advisory only. Navigation content identifies considerations and source limitations; it does not generate executable navigation orders or make navigational decisions.";

const finding = {
  classification: "OFFICIAL",
  text: "Review the verified priority.",
  sourceIds: ["ledger-1"],
};

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
] as const;

const sections = Object.fromEntries(sectionKeys.map((key) => [key, [finding]]));

const contributions = [
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
    missingInformation: [
      "Reminders permission is denied.",
      "World Monitor was unavailable for the Maritime N2 update.",
    ],
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
    ],
    sourceFreshness: {
      classification: "OFFICIAL",
      asOf: "2026-07-25T05:56:00Z",
      staleSourceIds: ["ledger-1"],
    },
    contributions,
    advisoryLimitation: ADVISORY_LIMITATION,
  },
};

const degradedStatus = {
  classification: "OFFICIAL",
  runId: "run-1",
  scheduleId: "daily-command-brief",
  sequence: 7,
  state: "degraded",
  updatedAt: "2026-07-25T06:10:00Z",
  degradedSections: ["navigation"],
  error: null,
};

test("Daily Command Brief opens in a truthful empty OFFICIAL state", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");

  await page.getByTestId("open-command-console-view").click();

  const consoleScreen = page.getByTestId("command-console-screen");
  await expect(consoleScreen).toBeVisible();
  await expect(
    consoleScreen.getByTestId("command-console-official-banner"),
  ).toContainText("OFFICIAL");
  await expect(
    consoleScreen.getByRole("heading", {
      name: "Daily Command Brief",
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    consoleScreen.getByText("No Daily Command Brief has been generated."),
  ).toBeVisible();
  await expect(
    consoleScreen.getByRole("button", { name: "Generate Daily Brief" }),
  ).toBeEnabled();
  await expect(consoleScreen.getByText("Not yet operational")).toHaveCount(0);
});

test("renders a complete degraded brief with retained evidence boundaries", async ({
  page,
}) => {
  await installMockBridge(page, {
    commandBriefLatest: published,
    commandBriefStatus: {
      classification: "OFFICIAL",
      current: degradedStatus,
      history: [degradedStatus],
    },
  });
  await page.goto("/");
  await page.getByTestId("open-command-console-view").click();

  const brief = page.getByTestId("daily-command-brief");
  await expect(brief).toBeVisible();

  for (const heading of [
    "Decisions and approvals required",
    "Today at a glance",
    "Operational priorities and risks",
    "Intelligence and operating environment",
    "Logistics and sustainment",
    "Navigation considerations",
    "Daily routine and calendar",
    "Reports and returns due",
    "30, 60 and 90-day outlook",
  ]) {
    await expect(
      brief.getByText(heading, { exact: true }).first(),
    ).toBeVisible();
  }

  const headings = brief.locator("[data-testid='brief-main-sections'] h3");
  await expect(headings).toHaveText([
    "Decisions and approvals required",
    "Today at a glance",
    "Operational priorities and risks",
    "Intelligence and operating environment",
    "Logistics and sustainment",
    "Navigation considerations",
    "Daily routine and calendar",
    "Reports and returns due",
    "30, 60 and 90-day outlook",
  ]);
  await expect(
    brief
      .getByTestId("brief-main-sections")
      .getByText("World Monitor was unavailable for the Maritime N2 update."),
  ).toHaveCount(0);
  await expect(
    brief.getByText(
      "World Monitor was unavailable for the Maritime N2 update.",
    ),
  ).toBeHidden();

  const disclosure = brief.getByTestId("brief-evidence-disclosure");
  await expect(disclosure).not.toHaveAttribute("open", "");
  await expect(brief.getByText("Source ledger", { exact: true })).toBeHidden();
  await expect(
    brief.getByText("Specialist adviser contributions", { exact: true }),
  ).toBeHidden();
  await expect(brief.getByText("snapshot-verified").first()).toBeHidden();
  await expect(brief.getByTestId("command-status-rag")).toBeHidden();

  const citation = brief
    .locator('a[href="#command-brief-source-ledger-1"]')
    .first();
  await expect(citation).toBeVisible();
  await citation.click();

  await expect(disclosure).toHaveAttribute("open", "");
  await expect(
    brief.getByText("Relay publication queued — offline capable"),
  ).toBeVisible();
  await expect(
    brief.getByText("Reminders permission is denied.").first(),
  ).toBeVisible();
  await expect(
    brief.getByText(
      "World Monitor was unavailable for the Maritime N2 update.",
    ),
  ).toBeVisible();
  await expect(
    brief
      .getByTestId("brief-main-sections")
      .getByText("Plans retains a dissenting view."),
  ).toBeVisible();
  await expect(
    brief.getByText("Chart update age requires review.").first(),
  ).toBeVisible();
  await expect(brief.getByText(ADVISORY_LIMITATION)).toBeVisible();
  await expect(
    brief.getByText("Prepare a workspace checklist proposal.").first(),
  ).toBeVisible();
  await expect(brief.getByText("Pending proposal").first()).toBeVisible();
  await expect(brief.getByText("snapshot-verified").first()).toBeVisible();
  await expect(brief.getByText("Stale source")).toBeVisible();
  await expect(brief.getByTestId("command-status-rag")).toBeVisible();

  const citedSource = brief.locator("#command-brief-source-ledger-1");
  await expect(citedSource).toBeFocused();
  await expect(
    citedSource.locator('time[datetime="2026-07-25T05:55:00Z"]'),
  ).toBeVisible();
  await expect(citedSource.getByText("page 7", { exact: true })).toBeVisible();
  await expect(brief.getByText("hidden evidence")).toHaveCount(0);
  await expect(brief.getByText(/approve|execute action/i)).toHaveCount(0);
});
