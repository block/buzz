import { expect, test } from "@playwright/test";

import {
  installMockBridge,
  type MockAgentUsage,
  type MockAgentUsageSeries,
} from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

const SHOTS = "test-results/agent-usage-screenshots";

const usageField = (value: string | null, incomplete = false) => ({
  value,
  incomplete,
});
const costField = (value: number | null, incomplete = false) => ({
  value,
  incomplete,
});

function reportedUsage(
  overrides: Partial<{
    inputTokens: string | null;
    outputTokens: string | null;
    totalTokens: string | null;
    estimatedCostUsd: number | null;
    cacheReadTokens: string | null;
    cacheWriteTokens: string | null;
    freshInputTokens: string | null;
  }> = {},
) {
  return {
    estimatedCostUsd: costField(overrides.estimatedCostUsd ?? null),
    inputTokens: usageField(overrides.inputTokens ?? null),
    outputTokens: usageField(overrides.outputTokens ?? null),
    totalTokens: usageField(overrides.totalTokens ?? null),
    cacheReadTokens: usageField(overrides.cacheReadTokens ?? null),
    cacheWriteTokens: usageField(overrides.cacheWriteTokens ?? null),
    freshInputTokens: usageField(overrides.freshInputTokens ?? null),
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
    usage: reportedUsage({ inputTokens: "1200", outputTokens: "300" }),
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
      (window as Window & { __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: unknown })
        .__BUZZ_E2E_INVOKE_MOCK_COMMAND__,
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

async function seedSeries(
  page: import("@playwright/test").Page,
  series: MockAgentUsageSeries,
) {
  await page.evaluate((next) => {
    const w = window as Window & {
      __BUZZ_E2E__?: { mock?: { agentUsageSeries?: unknown } };
    };
    w.__BUZZ_E2E__ ??= {};
    w.__BUZZ_E2E__.mock ??= {};
    w.__BUZZ_E2E__.mock.agentUsageSeries = next;
  }, series);
  await page.getByTestId("open-agents-view").click();
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

  test("01-overview-usage-section", async ({ page }) => {
    await installMockBridge(page);
    await openAgentsView(page);

    const agentPubkey = await addGenericAgent(page, "Usage Bot");

    // Four buckets: known, partial/unknown, gap (—), and zero — mirrors the
    // daily-bars accessible-label test in agent-usage.spec.ts.
    const base = 1_700_000_000;
    await seedSeries(
      page,
      mockUsageSeries({
        agents: [
          mockAgentUsage(agentPubkey, {
            hasUnknownUsage: true,
            usage: {
              estimatedCostUsd: costField(null),
              inputTokens: usageField(null),
              outputTokens: usageField(null),
              totalTokens: usageField("1500", true),
              cacheReadTokens: usageField(null),
              cacheWriteTokens: usageField(null),
              freshInputTokens: usageField(null),
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
    );

    await expect(page.getByTestId("agent-usage-card")).toBeVisible();
    await expect(
      page.getByTestId(`agent-usage-row-${agentPubkey}`),
    ).toBeVisible();

    const card = page.getByTestId("agent-usage-card");
    await waitForAnimations(page);
    await card.screenshot({ path: `${SHOTS}/01-overview-usage-section.png` });
  });

  test("02-focused-usage-view", async ({ page }) => {
    await installMockBridge(page);
    await openAgentsView(page);

    const agentPubkey = await addGenericAgent(page, "Drilldown Bot");

    const bucketStart = 1_700_000_000;
    await seedSeries(
      page,
      mockUsageSeries({
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
                harness: "claude-code",
                hasUnknownUsage: false,
                model: "claude-opus-4-5",
                reportCount: 4,
                usage: reportedUsage({
                  totalTokens: "2800",
                  inputTokens: "2100",
                  outputTokens: "700",
                  estimatedCostUsd: 0.35,
                  cacheReadTokens: "1400",
                  cacheWriteTokens: "300",
                  freshInputTokens: "400",
                }),
              },
              {
                harness: "goose",
                hasUnknownUsage: false,
                model: "claude-sonnet-4-5",
                reportCount: 1,
                usage: reportedUsage({
                  totalTokens: "800",
                  inputTokens: "600",
                  outputTokens: "200",
                  estimatedCostUsd: 0.04,
                  cacheReadTokens: "350",
                  cacheWriteTokens: "50",
                  freshInputTokens: "200",
                }),
              },
            ],
            usage: reportedUsage({
              estimatedCostUsd: 0.39,
              inputTokens: "2700",
              outputTokens: "900",
              totalTokens: "3600",
              cacheReadTokens: "1750",
              cacheWriteTokens: "350",
              freshInputTokens: "600",
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
    );

    await expect(
      page.getByTestId(`agent-usage-row-${agentPubkey}`),
    ).toBeVisible();

    // Click the row to open the focused view.
    await page.getByTestId(`agent-usage-row-${agentPubkey}`).click();
    await expect(page.getByTestId("user-profile-panel")).toBeVisible();
    await expect(page.getByTestId("agent-usage-focused-view")).toBeVisible();

    await expect(
      page.getByTestId("agent-usage-model-harness-label").first(),
    ).toBeVisible();

    const panel = page.getByTestId("user-profile-panel");
    await waitForAnimations(page);
    await panel.screenshot({ path: `${SHOTS}/02-focused-usage-view.png` });
  });

  test("03-custom-range-picker", async ({ page }) => {
    await installMockBridge(page, { agentUsageSeries: mockUsageSeries() });
    await openAgentsView(page);

    await page.getByTestId("agent-usage-window-custom").click();
    await expect(
      page.getByTestId("agent-usage-window-custom-popover"),
    ).toBeVisible();
    await page
      .getByTestId("agent-usage-window-custom-start")
      .fill("2026-01-05");
    await page.getByTestId("agent-usage-window-custom-end").fill("2026-01-19");
    await expect(
      page.getByTestId("agent-usage-window-custom-summary"),
    ).toBeVisible();

    await waitForAnimations(page);
    // Full-page shot: the popover is portalled outside the card element.
    await page.screenshot({ path: `${SHOTS}/03-custom-range-picker.png` });
  });
});
