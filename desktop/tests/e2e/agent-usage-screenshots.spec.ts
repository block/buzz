/**
 * Screenshot spec for the Agent Usage UI (PR #2035).
 *
 * Captures the four user-facing surfaces added by this PR:
 *   1. Overview card (collapsed/default) on the Agents page — daily bars + agent row
 *   2. Focused usage subview in the agent profile panel (expanded)
 *   3. Multi-day bars chart with varied data (known, partial, unknown, empty days)
 *   4. Empty state when collection is on but nothing has been archived yet
 */

import { expect, test } from "@playwright/test";

import {
  installMockBridge,
  type MockAgentUsage,
  type MockAgentUsageSeries,
} from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

const SHOTS = "test-results/agent-usage-screenshots";

function usageField(value: string | null, incomplete = false) {
  return { value, incomplete };
}

function costField(value: number | null, incomplete = false) {
  return { value, incomplete };
}

function reportedUsage(
  overrides: Partial<{
    inputTokens: string | null;
    outputTokens: string | null;
    totalTokens: string | null;
    estimatedCostUsd: number | null;
  }> = {},
) {
  return {
    estimatedCostUsd: costField(overrides.estimatedCostUsd ?? null),
    inputTokens: usageField(overrides.inputTokens ?? null),
    outputTokens: usageField(overrides.outputTokens ?? null),
    totalTokens: usageField(overrides.totalTokens ?? null),
  };
}

function mockAgentUsage(
  agentPubkey: string,
  overrides: Partial<MockAgentUsage> = {},
): MockAgentUsage {
  return {
    agentPubkey,
    buckets: [],
    hasUnknownUsage: false,
    models: [],
    reportCount: 1,
    usage: reportedUsage({ totalTokens: "1500" }),
    ...overrides,
  };
}

function mockUsageSeries(
  overrides: Partial<MockAgentUsageSeries> = {},
): MockAgentUsageSeries {
  return {
    agents: [],
    buckets: [],
    collectionEnabled: true,
    coverage: {
      firstArchivedAt: null,
      firstReportedAt: null,
      hasUnknownUsage: false,
      invalidReportCount: 0,
      lastArchivedAt: null,
      lastReportedAt: null,
      reportCount: 0,
    },
    hasArchivedEvidence: null,
    ...overrides,
  };
}

async function openAgentsView(page: import("@playwright/test").Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-agents-view").click();
  await expect(page.getByTestId("agents-usage-section")).toBeVisible({
    timeout: 10_000,
  });
}

async function addGenericAgent(
  page: import("@playwright/test").Page,
  agentName: string,
): Promise<string> {
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  const channelId = await page
    .getByTestId("channel-general")
    .getAttribute("data-channel-id");
  if (!channelId) throw new Error("channel-general is missing data-channel-id");

  await page.waitForFunction(() =>
    Boolean(
      (
        window as Window & {
          __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: unknown;
        }
      ).__BUZZ_E2E_INVOKE_MOCK_COMMAND__,
    ),
  );

  return page.evaluate(
    async ({ agentName, channelId }): Promise<string> => {
      const invoke = (
        window as Window & {
          __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
            command: string,
            payload?: Record<string, unknown>,
          ) => Promise<{ agent?: { pubkey: string } }>;
        }
      ).__BUZZ_E2E_INVOKE_MOCK_COMMAND__;
      if (!invoke) throw new Error("Mock bridge not installed.");

      const created = await invoke("create_managed_agent", {
        input: {
          name: agentName,
          spawnAfterCreate: true,
          systemPrompt: "Help when asked.",
        },
      });
      const pubkey = created.agent?.pubkey;
      if (!pubkey)
        throw new Error("create_managed_agent did not return pubkey");

      await invoke("add_channel_members", {
        channelId,
        pubkeys: [pubkey],
        role: "bot",
      });

      await (
        window as Window & {
          __BUZZ_E2E_QUERY_CLIENT__?: {
            invalidateQueries: () => Promise<void>;
          };
        }
      ).__BUZZ_E2E_QUERY_CLIENT__?.invalidateQueries();

      return pubkey;
    },
    { agentName, channelId },
  );
}

test.describe("agent usage screenshots", () => {
  test.use({ viewport: { width: 1280, height: 900 } });

  test.beforeEach(async ({ page }) => {
    page.on("pageerror", (err) => {
      console.error(
        "PAGE ERROR:",
        err.message,
        err.stack?.split("\n").slice(0, 5).join("\n"),
      );
    });
  });

  // ── Shot 1: Overview card (collapsed/default) with daily bars + agent row ──
  test("01-overview-usage-section", async ({ page }) => {
    await installMockBridge(page);
    await openAgentsView(page);

    const agentPubkey = await addGenericAgent(page, "Usage Bot");

    // Four buckets: known, partial/unknown, gap (—), and zero — mirrors the
    // daily-bars accessible-label test in agent-usage.spec.ts.
    const base = 1_700_000_000;
    await page.evaluate(
      ({ series }) => {
        const w = window as Window & {
          __BUZZ_E2E__?: { mock?: { agentUsageSeries?: unknown } };
        };
        w.__BUZZ_E2E__ ??= {};
        w.__BUZZ_E2E__.mock ??= {};
        w.__BUZZ_E2E__.mock.agentUsageSeries = series;
      },
      {
        series: mockUsageSeries({
          agents: [
            mockAgentUsage(agentPubkey, {
              hasUnknownUsage: true,
              usage: {
                estimatedCostUsd: costField(null),
                inputTokens: usageField(null),
                outputTokens: usageField(null),
                totalTokens: usageField("1500", true),
              },
            }),
          ],
          buckets: [
            {
              start: base,
              end: base + 86_400,
              usage: reportedUsage({ totalTokens: "700" }),
              reportCount: 1,
              hasUnknownUsage: false,
            },
            {
              start: base + 86_400,
              end: base + 2 * 86_400,
              usage: reportedUsage({ totalTokens: null }),
              reportCount: 1,
              hasUnknownUsage: true,
            },
            {
              start: base + 2 * 86_400,
              end: base + 3 * 86_400,
              usage: reportedUsage({ totalTokens: null }),
              reportCount: 0,
              hasUnknownUsage: false,
            },
            {
              start: base + 3 * 86_400,
              end: base + 4 * 86_400,
              usage: reportedUsage({ totalTokens: "0" }),
              reportCount: 1,
              hasUnknownUsage: false,
            },
          ],
          coverage: {
            firstArchivedAt: base,
            firstReportedAt: base,
            hasUnknownUsage: true,
            invalidReportCount: 0,
            lastArchivedAt: base + 3 * 86_400,
            lastReportedAt: base + 3 * 86_400,
            reportCount: 3,
          },
        }),
      },
    );

    await page.getByTestId("open-agents-view").click();
    await expect(page.getByTestId("agent-usage-card")).toBeVisible();
    await expect(
      page.getByTestId(`agent-usage-row-${agentPubkey}`),
    ).toBeVisible();

    const card = page.getByTestId("agent-usage-card");
    await waitForAnimations(page);
    await card.screenshot({ path: `${SHOTS}/01-overview-usage-section.png` });
  });

  // ── Shot 2: Focused usage subview (expanded) with totals + model breakdown ──
  test("02-focused-usage-view", async ({ page }) => {
    await installMockBridge(page);
    await openAgentsView(page);

    const agentPubkey = await addGenericAgent(page, "Drilldown Bot");

    const bucketStart = 1_700_000_000;
    await page.evaluate(
      ({ series }) => {
        const w = window as Window & {
          __BUZZ_E2E__?: { mock?: { agentUsageSeries?: unknown } };
        };
        w.__BUZZ_E2E__ ??= {};
        w.__BUZZ_E2E__.mock ??= {};
        w.__BUZZ_E2E__.mock.agentUsageSeries = series;
      },
      {
        series: mockUsageSeries({
          agents: [
            mockAgentUsage(agentPubkey, {
              reportCount: 5,
              buckets: [
                {
                  start: bucketStart,
                  end: bucketStart + 86_400,
                  usage: reportedUsage({
                    totalTokens: "2400",
                    inputTokens: "1800",
                    outputTokens: "600",
                  }),
                  reportCount: 3,
                  hasUnknownUsage: false,
                },
                {
                  start: bucketStart + 86_400,
                  end: bucketStart + 2 * 86_400,
                  usage: reportedUsage({
                    totalTokens: "1200",
                    inputTokens: "900",
                    outputTokens: "300",
                  }),
                  reportCount: 2,
                  hasUnknownUsage: false,
                },
              ],
              models: [
                {
                  hasUnknownUsage: false,
                  model: "claude-opus-4-5",
                  reportCount: 4,
                  usage: reportedUsage({
                    totalTokens: "2800",
                    inputTokens: "2100",
                    outputTokens: "700",
                    estimatedCostUsd: 0.35,
                  }),
                },
                {
                  hasUnknownUsage: false,
                  model: "claude-sonnet-4-5",
                  reportCount: 1,
                  usage: reportedUsage({
                    totalTokens: "800",
                    inputTokens: "600",
                    outputTokens: "200",
                    estimatedCostUsd: 0.04,
                  }),
                },
              ],
              usage: reportedUsage({
                estimatedCostUsd: 0.39,
                inputTokens: "2700",
                outputTokens: "900",
                totalTokens: "3600",
              }),
            }),
          ],
          coverage: {
            firstArchivedAt: bucketStart,
            firstReportedAt: bucketStart,
            hasUnknownUsage: false,
            invalidReportCount: 0,
            lastArchivedAt: bucketStart + 2 * 86_400,
            lastReportedAt: bucketStart + 2 * 86_400,
            reportCount: 5,
          },
        }),
      },
    );

    await page.getByTestId("open-agents-view").click();
    await expect(
      page.getByTestId(`agent-usage-row-${agentPubkey}`),
    ).toBeVisible();

    // Click the row to open the focused view.
    await page.getByTestId(`agent-usage-row-${agentPubkey}`).click();
    await expect(page.getByTestId("user-profile-panel")).toBeVisible();
    await expect(page.getByTestId("agent-usage-focused-view")).toBeVisible();

    const panel = page.getByTestId("user-profile-panel");
    await waitForAnimations(page);
    await panel.screenshot({ path: `${SHOTS}/02-focused-usage-view.png` });
  });

  // ── Shot 3: Daily bars overview — 7+ days of varied data ──────────────────
  test("03-daily-bars-multi-day", async ({ page }) => {
    await installMockBridge(page);
    await openAgentsView(page);

    const agentPubkey = await addGenericAgent(page, "Bar Bot");

    const base = 1_700_000_000;
    const days = [900, 1200, 450, 1800, 600, 0, 1100, 750];
    const buckets = days.map((tokens, i) => ({
      start: base + i * 86_400,
      end: base + (i + 1) * 86_400,
      usage: reportedUsage({ totalTokens: tokens > 0 ? String(tokens) : null }),
      reportCount: tokens > 0 ? 1 : 0,
      hasUnknownUsage: false,
    }));

    await page.evaluate(
      ({ series }) => {
        const w = window as Window & {
          __BUZZ_E2E__?: { mock?: { agentUsageSeries?: unknown } };
        };
        w.__BUZZ_E2E__ ??= {};
        w.__BUZZ_E2E__.mock ??= {};
        w.__BUZZ_E2E__.mock.agentUsageSeries = series;
      },
      {
        series: mockUsageSeries({
          agents: [
            mockAgentUsage(agentPubkey, {
              usage: reportedUsage({
                totalTokens: "6800",
                inputTokens: "5100",
                outputTokens: "1700",
              }),
            }),
          ],
          buckets,
          coverage: {
            firstArchivedAt: base,
            firstReportedAt: base,
            hasUnknownUsage: false,
            invalidReportCount: 0,
            lastArchivedAt: base + 7 * 86_400,
            lastReportedAt: base + 7 * 86_400,
            reportCount: 6,
          },
        }),
      },
    );

    await page.getByTestId("open-agents-view").click();
    await expect(page.getByTestId("agent-usage-overall-bars")).toBeVisible();

    const card = page.getByTestId("agent-usage-card");
    await waitForAnimations(page);
    await card.screenshot({
      path: `${SHOTS}/03-daily-bars-multi-day.png`,
    });
  });

  // ── Shot 4: Empty state — collection on, nothing archived yet ─────────────
  test("04-empty-state", async ({ page }) => {
    await installMockBridge(page, {
      agentUsageSeries: mockUsageSeries(),
    });

    await openAgentsView(page);

    const empty = page.getByTestId("agent-usage-empty");
    await expect(empty).toBeVisible();
    await expect(empty).toContainText("No locally archived usage");

    const card = page.getByTestId("agent-usage-card");
    await waitForAnimations(page);
    await card.screenshot({ path: `${SHOTS}/04-empty-state.png` });
  });
});
