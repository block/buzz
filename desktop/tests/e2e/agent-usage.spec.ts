import { expect, test, type Page } from "@playwright/test";

import {
  installMockBridge,
  TEST_IDENTITIES,
  type MockAgentUsage,
  type MockAgentUsageSeries,
} from "../helpers/bridge";

const DAY = 86_400;
const BASE = 1_700_000_000;

/**
 * A non-owned, non-managed agent (declared owner is `outsider`, not the mock
 * viewer) — used by the A13 fail-closed tests where eligibility must come
 * from archived evidence alone, not ownership.
 */
const HISTORICAL_AGENT_PUBKEY = "6".repeat(64);

/**
 * A non-owned, non-managed agent seeded purely via `searchProfiles` (no
 * managed-agent creation) — used by the ingress-visibility test's non-owner
 * leg, which must be hidden regardless of archived evidence.
 */
const NON_OWNER_AGENT_PUBKEY = "7".repeat(64);

function getHashSearchParam(page: Page, name: string) {
  const hash = new URL(page.url()).hash.replace(/^#/, "");
  const queryStart = hash.indexOf("?");
  if (queryStart === -1) {
    return null;
  }
  return new URLSearchParams(hash.slice(queryStart + 1)).get(name);
}

async function expectHashSearchParam(
  page: Page,
  name: string,
  value: string | null,
) {
  await expect.poll(() => getHashSearchParam(page, name)).toBe(value);
}

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
    // Default: i/o known, no totalTokens — real prod shape, exercises ≈ path.
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

/** A fully-populated coverage block for a window that did archive evidence. */
function archivedCoverage(reportCount: number) {
  return {
    firstArchivedAt: BASE,
    firstReportedAt: BASE,
    hasUnknownUsage: false,
    invalidReportCount: 0,
    lastArchivedAt: BASE + DAY,
    lastReportedAt: BASE + DAY,
    reportCount,
  };
}

function dayBucket(
  dayIndex: number,
  usage: ReturnType<typeof reportedUsage>,
  overrides: Partial<{ reportCount: number; hasUnknownUsage: boolean }> = {},
) {
  return {
    start: BASE + dayIndex * DAY,
    end: BASE + (dayIndex + 1) * DAY,
    usage,
    reportCount: overrides.reportCount ?? 1,
    hasUnknownUsage: overrides.hasUnknownUsage ?? false,
  };
}

async function addGenericAgent(
  page: Page,
  channelName: string,
  agentName: string,
): Promise<string> {
  await page.getByTestId(`channel-${channelName}`).click();
  await expect(page.getByTestId("chat-title")).toHaveText(channelName);
  const channelId = await page
    .getByTestId(`channel-${channelName}`)
    .getAttribute("data-channel-id");
  if (!channelId) {
    throw new Error(`Channel ${channelName} is missing a data-channel-id.`);
  }

  await page.waitForFunction(() => {
    return Boolean(
      (window as Window & { __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: unknown })
        .__BUZZ_E2E_INVOKE_MOCK_COMMAND__,
    );
  });
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
      if (!invoke) {
        throw new Error("Mock bridge is not installed.");
      }

      const created = (await invoke("create_managed_agent", {
        input: {
          name: agentName,
          spawnAfterCreate: true,
          systemPrompt: "Watch the channel and help when asked.",
        },
      })) as { agent?: { pubkey: string } };
      const pubkey = created.agent?.pubkey;
      if (!pubkey) {
        throw new Error("Mock managed agent creation did not return a pubkey.");
      }

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

async function openAgentsView(page: Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-agents-view").click();
  await expect(page.getByTestId("agents-usage-section")).toBeVisible({
    timeout: 10_000,
  });
}

/** Swap in a usage series and re-enter the Agents view so it is queried. */
async function seedSeries(page: Page, series: MockAgentUsageSeries) {
  await page.evaluate((next) => {
    const testWindow = window as Window & {
      __BUZZ_E2E__?: { mock?: { agentUsageSeries?: unknown } };
    };
    testWindow.__BUZZ_E2E__ ??= {};
    testWindow.__BUZZ_E2E__.mock ??= {};
    testWindow.__BUZZ_E2E__.mock.agentUsageSeries = next;
  }, series);
  await page.getByTestId("open-agents-view").click();
  await expect(page.getByTestId("agents-usage-section")).toBeVisible();
}

async function navigateToProfile(
  page: Page,
  pubkey: string,
  profileView?: "usage",
) {
  await page.evaluate(
    ({ profileView, pubkey }) => {
      (
        window as Window & {
          __TSR_ROUTER__?: {
            navigate: (opts: Record<string, unknown>) => void;
          };
        }
      ).__TSR_ROUTER__?.navigate({
        to: "/agents",
        search: profileView
          ? { profile: pubkey, profileView }
          : { profile: pubkey },
      });
    },
    { profileView, pubkey },
  );
  await expect(page.getByTestId("user-profile-panel")).toBeVisible({
    timeout: 10_000,
  });
}

test("shows a loading skeleton while the usage series is in flight", async ({
  page,
}) => {
  await installMockBridge(page, {
    agentUsageSeries: mockUsageSeries(),
    agentUsageDelayMs: 2_000,
  });

  await openAgentsView(page);

  await expect(page.getByTestId("agent-usage-skeleton")).toBeVisible();
  await expect(page.getByTestId("agent-usage-card")).toBeVisible({
    timeout: 5_000,
  });
  await expect(page.getByTestId("agent-usage-skeleton")).toHaveCount(0);
});

test("renders ranked agent rows and switches between the 1d, 7d, and 30d presets", async ({
  page,
}) => {
  await installMockBridge(page);
  await openAgentsView(page);

  const agentPubkey = await addGenericAgent(page, "general", "Token Bot");
  await seedSeries(
    page,
    mockUsageSeries({
      agents: [
        mockAgentUsage(agentPubkey, {
          usage: reportedUsage({
            inputTokens: "1200",
            outputTokens: "300",
            totalTokens: "1500",
          }),
        }),
      ],
      coverage: archivedCoverage(1),
    }),
  );

  const row = page.getByTestId(`agent-usage-row-${agentPubkey}`);
  await expect(row).toBeVisible();
  await expect(row).toContainText("1.5K");
  await expect(row).toContainText("Token Bot");

  await expect(page.getByTestId("agent-usage-window-7")).toHaveAttribute(
    "data-state",
    "active",
  );

  for (const preset of ["30", "1"]) {
    await page.getByTestId(`agent-usage-window-${preset}`).click();
    await expect(
      page.getByTestId(`agent-usage-window-${preset}`),
    ).toHaveAttribute("data-state", "active");
    // Switching presets must never blank the card.
    await expect(page.getByTestId("agent-usage-card")).toBeVisible();
  }
});

test("clicking an agent row opens the profile panel's Usage focused view", async ({
  page,
}) => {
  await installMockBridge(page);
  await openAgentsView(page);

  const agentPubkey = await addGenericAgent(page, "general", "Drilldown Bot");
  await seedSeries(
    page,
    mockUsageSeries({
      agents: [
        mockAgentUsage(agentPubkey, {
          models: [
            {
              harness: "goose",
              hasUnknownUsage: false,
              model: "claude-opus",
              reportCount: 1,
              usage: reportedUsage({ totalTokens: "1500" }),
            },
          ],
          usage: reportedUsage({
            estimatedCostUsd: 0.42,
            inputTokens: "1200",
            outputTokens: "300",
            totalTokens: "1500",
          }),
        }),
      ],
    }),
  );

  await page.getByTestId(`agent-usage-row-${agentPubkey}`).click();
  await expect(page.getByTestId("user-profile-panel")).toBeVisible();
  await expect(page.getByTestId("agent-usage-focused-view")).toBeVisible();
  await expect(page.getByTestId("agent-usage-focused-totals")).toContainText(
    "1,500",
  );
  const models = page.getByTestId("agent-usage-focused-models");
  await expect(models).toContainText("claude-opus");
  await expect(models).toContainText("goose");

  // Also verify the Info-tab ingress row entry point, not just row-click.
  await page.getByTestId("user-profile-panel-back").click();
  await expect(page.getByTestId("user-profile-tab-info")).toBeVisible();
  await page.getByTestId(`user-profile-view-usage-${agentPubkey}`).click();
  await expect(page.getByTestId("agent-usage-focused-view")).toBeVisible();
});

test("Info-tab Usage ingress is visible for an owner-viewed agent, and absent for a human or a non-owner agent", async ({
  page,
}) => {
  await installMockBridge(page, {
    agentUsageSeries: mockUsageSeries(),
    searchProfiles: [
      {
        pubkey: NON_OWNER_AGENT_PUBKEY,
        displayName: "Someone Else's Bot",
        isAgent: true,
        ownerPubkey: TEST_IDENTITIES.outsider.pubkey,
      },
    ],
  });

  // Owner case: locally-managed agent is owned by the mock viewer → canViewUsage true.
  await openAgentsView(page);
  const ownedAgentPubkey = await addGenericAgent(page, "general", "Own Bot");
  await seedSeries(
    page,
    mockUsageSeries({ agents: [mockAgentUsage(ownedAgentPubkey)] }),
  );
  await page.getByTestId(`agent-usage-row-${ownedAgentPubkey}`).click();
  await expect(page.getByTestId("user-profile-panel")).toBeVisible();
  await page.getByTestId("user-profile-panel-back").click();
  await expect(page.getByTestId("user-profile-tab-info")).toBeVisible();
  await expect(
    page.getByTestId(`user-profile-view-usage-${ownedAgentPubkey}`),
  ).toBeVisible();

  // Human case: canViewUsage requires isBot — row is absent for any human profile.
  await navigateToProfile(page, TEST_IDENTITIES.bob.pubkey);
  await expect(
    page.getByTestId(`user-profile-view-usage-${TEST_IDENTITIES.bob.pubkey}`),
  ).toHaveCount(0);

  // Non-owner agent case: isBot=true but viewerIsOwner=false → row stays hidden.
  await navigateToProfile(page, NON_OWNER_AGENT_PUBKEY);
  await expect(
    page.getByTestId(`user-profile-view-usage-${NON_OWNER_AGENT_PUBKEY}`),
  ).toHaveCount(0);
});

test("surfaces a retry affordance when the usage query fails, and recovers on retry", async ({
  page,
}) => {
  // React Query auto-retries once (queryClient.ts `retry: 1`), consuming one
  // sequence entry silently — two failures are needed before the error UI shows.
  await installMockBridge(page, {
    agentUsageErrors: ["archive unavailable", "archive unavailable", null],
  });

  await openAgentsView(page);

  const error = page.getByTestId("agent-usage-error");
  await expect(error).toBeVisible();
  await expect(error).toContainText("Couldn't load usage data.");

  await error.getByRole("button", { name: "Retry" }).click();

  await expect(page.getByTestId("agent-usage-card")).toBeVisible();
  await expect(page.getByTestId("agent-usage-error")).toHaveCount(0);
});

test("shows the empty state when collection is on but nothing has been archived yet", async ({
  page,
}) => {
  await installMockBridge(page, {
    agentUsageSeries: mockUsageSeries(),
  });

  await openAgentsView(page);

  const empty = page.getByTestId("agent-usage-empty");
  await expect(empty).toBeVisible();
  await expect(empty).toContainText("No locally archived usage");
  await expect(page.getByTestId("agent-usage-collection-off")).toHaveCount(0);
});

test("shows the collection-off banner with a settings deep link", async ({
  page,
}) => {
  await installMockBridge(page, {
    agentUsageSeries: mockUsageSeries({ collectionEnabled: false }),
  });

  await openAgentsView(page);

  const banner = page.getByTestId("agent-usage-collection-off");
  await expect(banner).toBeVisible();
  await expect(banner).toContainText("Local usage collection is off.");
  await expect(page.getByTestId("agent-usage-empty")).toContainText(
    "Turn on collection",
  );

  await banner
    .getByRole("button", { name: "Open Local Archive settings" })
    .click();
  await expect(page.getByTestId("settings-view")).toBeVisible();
  await expect(page.getByTestId("settings-local-archive")).toBeVisible({
    timeout: 10_000,
  });

  await page.goBack();
  await expect(page.getByTestId("agents-usage-section")).toBeVisible();
});

// A13 fail-closed: focused view eligibility = ownership OR archived evidence,
// resolved once the author-filtered query completes. These tests deep-link
// straight to `?profileView=usage` for a non-owned agent to bypass ownership.
async function openUsageViewForHistoricalAgent(
  page: Page,
  agentUsageSeries: MockAgentUsageSeries,
) {
  await installMockBridge(page, {
    agentUsageSeries,
    searchProfiles: [
      {
        pubkey: HISTORICAL_AGENT_PUBKEY,
        displayName: "Historical Bot",
        isAgent: true,
        ownerPubkey: TEST_IDENTITIES.outsider.pubkey,
      },
    ],
  });
  await page.goto("/#/agents", { waitUntil: "domcontentloaded" });
  await expect(page.getByTestId("agents-usage-section")).toBeVisible({
    timeout: 10_000,
  });
  await navigateToProfile(page, HISTORICAL_AGENT_PUBKEY, "usage");
}

test("a historical author with 30d-only archived evidence gets a valid empty 7d focused view, not a redirect", async ({
  page,
}) => {
  await openUsageViewForHistoricalAgent(
    page,
    mockUsageSeries({ agents: [], hasArchivedEvidence: true }),
  );

  const outsideWindow = page.getByTestId("agent-usage-focused-outside-window");
  await expect(outsideWindow).toBeVisible();
  await expect(outsideWindow).toContainText("Try a wider window.");

  // Eligible via archived evidence alone (no ownership) — never bounced.
  await expectHashSearchParam(page, "profileView", "usage");
  const panel = page.getByTestId("user-profile-panel");
  await expect(
    panel.getByRole("heading", { level: 2, name: "Usage" }),
  ).toBeVisible();
});

test("a hand-authored usage URL with no ownership and no archived evidence falls back to summary", async ({
  page,
}) => {
  await openUsageViewForHistoricalAgent(
    page,
    mockUsageSeries({ agents: [], hasArchivedEvidence: null }),
  );

  // The redirect from "usage" → null may fire before the first poll
  // observes profileView=usage — skip that transient assertion; the null
  // landing + summary heading carry the contract.
  await expectHashSearchParam(page, "profileView", null);
  await expect(page.getByTestId("agent-usage-focused-view")).toHaveCount(0);
  const panel = page.getByTestId("user-profile-panel");
  await expect(
    panel.getByRole("heading", { level: 2, name: "Profile" }),
  ).toBeVisible();
});

test("daily bars label the date on the axis and the total on the bar, and their tooltips report unknown fields as unknown", async ({
  page,
}) => {
  await installMockBridge(page);
  await openAgentsView(page);

  const agentPubkey = await addGenericAgent(page, "general", "Axis Bot");

  const knownStart = BASE;
  const unknownStart = BASE + DAY;
  await seedSeries(
    page,
    mockUsageSeries({
      agents: [mockAgentUsage(agentPubkey, { buckets: [] })],
      buckets: [
        dayBucket(
          0,
          reportedUsage({
            inputTokens: "1200",
            outputTokens: "300",
            totalTokens: "1500",
          }),
        ),
        dayBucket(
          1,
          reportedUsage({
            inputTokens: "1200",
            outputTokens: null,
            totalTokens: null,
          }),
          { hasUnknownUsage: true },
        ),
      ],
    }),
  );
  await expect(page.getByTestId("agent-usage-overall-bars")).toBeVisible();

  // x-axis tick must be the DATE, not the token total — asserted via the same
  // toLocaleDateString call the component makes (locale/TZ-independent).
  const expectedDate = await page.evaluate(
    (start) =>
      new Date(start * 1000).toLocaleDateString(undefined, {
        month: "short",
        day: "numeric",
      }),
    knownStart,
  );
  const dateTick = page.getByTestId(`agent-usage-daily-bar-date-${knownStart}`);
  await expect(dateTick).toHaveText(expectedDate);
  // The pre-fix chart labeled the axis with the value — ensure that's not the case.
  await expect(dateTick).not.toContainText("1.5K");

  // Token total renders ON the bar; an uncountable day shows em-dash, not zero.
  await expect(
    page.getByTestId(`agent-usage-daily-bar-value-${knownStart}`),
  ).toHaveText("1.5K");
  await expect(
    page.getByTestId(`agent-usage-daily-bar-value-${unknownStart}`),
  ).toHaveText("—");

  // Aria labels: countable day → "reported tokens", unknown day → "unknown usage".
  const ariaLabel = (start: number) =>
    page
      .getByTestId(`agent-usage-daily-bar-${start}`)
      .locator("[aria-label]")
      .first()
      .getAttribute("aria-label");
  expect(await ariaLabel(knownStart)).toMatch(/reported tokens/i);
  expect(await ariaLabel(unknownStart)).toMatch(/unknown usage/i);

  // Hover tooltip: exact breakdown must show without opening the focused view.
  await page.getByTestId(`agent-usage-daily-bar-${knownStart}`).hover();
  const knownTooltip = page
    .getByTestId(`agent-usage-daily-bar-tooltip-${knownStart}`)
    .first();
  await expect(knownTooltip).toBeVisible({ timeout: 10_000 });
  await expect(knownTooltip).toContainText("Total: 1,500");
  await expect(knownTooltip).toContainText("Input: 1,200");
  await expect(knownTooltip).toContainText("Output: 300");
});

test("a bar tooltip reports an unknown field as unknown rather than zero", async ({
  page,
}) => {
  await installMockBridge(page);
  await openAgentsView(page);

  const agentPubkey = await addGenericAgent(page, "general", "Halfknown Bot");

  const bucketStart = BASE + DAY;
  await seedSeries(
    page,
    mockUsageSeries({
      agents: [mockAgentUsage(agentPubkey, { buckets: [] })],
      buckets: [
        // Output is genuinely unreported: it must read "unknown", and the
        // total must stay unknown rather than being derived from input.
        dayBucket(
          1,
          reportedUsage({
            inputTokens: "1200",
            outputTokens: null,
            totalTokens: null,
          }),
          { hasUnknownUsage: true },
        ),
      ],
    }),
  );
  await expect(page.getByTestId("agent-usage-overall-bars")).toBeVisible();

  await page.getByTestId(`agent-usage-daily-bar-${bucketStart}`).hover();
  const tooltip = page
    .getByTestId(`agent-usage-daily-bar-tooltip-${bucketStart}`)
    .first();
  await expect(tooltip).toBeVisible({ timeout: 10_000 });
  await expect(tooltip).toContainText("Total: unknown");
  await expect(tooltip).toContainText("Input: 1,200");
  await expect(tooltip).toContainText("Output: unknown");
  await expect(tooltip).not.toContainText("Output: 0");
});

// Regression guard: null totalTokens with known i/o must render ≈, not "No usage reported".
test("overview row, header, daily bar, and focused total all render ≈ when totalTokens is null but i/o is known", async ({
  page,
}) => {
  await installMockBridge(page);
  await openAgentsView(page);

  const agentPubkey = await addGenericAgent(page, "general", "Approx Bot");
  // Real prod shape — no provider total, both i/o known.
  const approxUsage = reportedUsage({
    inputTokens: "800",
    outputTokens: "200",
    totalTokens: null,
  });
  await seedSeries(
    page,
    mockUsageSeries({
      agents: [
        mockAgentUsage(agentPubkey, {
          buckets: [dayBucket(0, approxUsage)],
          usage: approxUsage,
        }),
      ],
      buckets: [dayBucket(0, approxUsage)],
      coverage: archivedCoverage(1),
    }),
  );

  const row = page.getByTestId(`agent-usage-row-${agentPubkey}`);
  await expect(row).toBeVisible();
  await expect(row).toContainText("≈");
  await expect(row).not.toContainText("No usage reported");

  // Assert value node directly so the header fails independently of daily bars.
  const headerValue = page.getByTestId("agent-usage-header-value");
  await expect(headerValue).toContainText("≈");
  await expect(headerValue).not.toContainText("No usage reported");

  await expect(page.getByTestId("agent-usage-daily-bars")).toContainText("≈");

  // Assert focused total value node directly so it fails independently.
  await row.click();
  await expect(page.getByTestId("agent-usage-focused-view")).toBeVisible();
  const focusedTotalValue = page.getByTestId("agent-usage-focused-total-value");
  await expect(focusedTotalValue).toContainText("≈");
  await expect(focusedTotalValue).not.toHaveText("—");
});
test("the custom range picker applies an arbitrary date span and labels it in the empty state", async ({
  page,
}) => {
  await installMockBridge(page, { agentUsageSeries: mockUsageSeries() });
  await openAgentsView(page);

  await page.getByTestId("agent-usage-window-custom").click();
  await expect(
    page.getByTestId("agent-usage-window-custom-popover"),
  ).toBeVisible();

  await page.getByTestId("agent-usage-window-custom-start").fill("2026-01-05");
  await page.getByTestId("agent-usage-window-custom-end").fill("2026-01-09");
  await expect(
    page.getByTestId("agent-usage-window-custom-summary"),
  ).toContainText("5 days selected");

  await page.getByTestId("agent-usage-window-custom-apply").click();
  await expect(
    page.getByTestId("agent-usage-window-custom-popover"),
  ).toHaveCount(0);

  // Custom tab is now active; empty-state names the applied span, not "last N days".
  await expect(page.getByTestId("agent-usage-window-custom")).toHaveAttribute(
    "data-state",
    "active",
  );
  const empty = page.getByTestId("agent-usage-empty");
  await expect(empty).toBeVisible();
  await expect(empty).toContainText("2026");
  await expect(empty).not.toContainText("the last 7 days");
});

test("the custom range picker blocks an inverted range and a span over one year", async ({
  page,
}) => {
  await installMockBridge(page, { agentUsageSeries: mockUsageSeries() });
  await openAgentsView(page);

  await page.getByTestId("agent-usage-window-custom").click();
  const apply = page.getByTestId("agent-usage-window-custom-apply");
  const error = page.getByTestId("agent-usage-window-custom-error");

  await page.getByTestId("agent-usage-window-custom-start").fill("2026-03-10");
  await page.getByTestId("agent-usage-window-custom-end").fill("2026-03-01");
  await expect(error).toContainText("on or before");
  await expect(apply).toBeDisabled();

  // 2024-01-01 → 2025-01-01 is 367 civil days: one past the cap.
  // The picker must block it locally, not let it surface a Rust error.
  await page.getByTestId("agent-usage-window-custom-start").fill("2024-01-01");
  await page.getByTestId("agent-usage-window-custom-end").fill("2025-01-01");
  await expect(error).toContainText("366 days or fewer");
  await expect(apply).toBeDisabled();

  // Pulling the end back to the cap clears the error and enables Apply.
  await page.getByTestId("agent-usage-window-custom-end").fill("2024-12-31");
  await expect(
    page.getByTestId("agent-usage-window-custom-summary"),
  ).toContainText("366 days selected");
  await expect(apply).toBeEnabled();
});

test("the focused view exposes its own independent range selector", async ({
  page,
}) => {
  await installMockBridge(page);
  await openAgentsView(page);

  const agentPubkey = await addGenericAgent(page, "general", "Focused Bot");
  await seedSeries(
    page,
    mockUsageSeries({ agents: [mockAgentUsage(agentPubkey)] }),
  );

  await page.getByTestId(`agent-usage-row-${agentPubkey}`).click();
  await expect(page.getByTestId("agent-usage-focused-view")).toBeVisible();

  await page.getByTestId("agent-usage-focused-window-1").click();
  await expect(
    page.getByTestId("agent-usage-focused-window-1"),
  ).toHaveAttribute("data-state", "active");

  // The overview's own selector is unaffected — the two windows are independent.
  await expect(page.getByTestId("agent-usage-window-7")).toHaveAttribute(
    "data-state",
    "active",
  );
});

test("the usage section renders below the agents and teams sections", async ({
  page,
}) => {
  await installMockBridge(page);
  await openAgentsView(page);

  const order = await page.evaluate(() => {
    const testIds = [
      "agents-library-personas",
      "agents-library-teams",
      "agents-usage-section",
    ];
    return testIds.map((testId) => {
      const element = document.querySelector(`[data-testid="${testId}"]`);
      return element === null
        ? null
        : element.getBoundingClientRect().top + window.scrollY;
    });
  });

  const [personasTop, teamsTop, usageTop] = order;
  expect(personasTop).not.toBeNull();
  expect(teamsTop).not.toBeNull();
  expect(usageTop).not.toBeNull();
  expect(usageTop as number).toBeGreaterThan(personasTop as number);
  expect(usageTop as number).toBeGreaterThan(teamsTop as number);
});

// ── Branch-coverage tests (restored compact form after trim, per Thufir pass-1) ─

test("Partial badge renders in the row, bar, and ingress when total is an incomplete lower bound", async ({
  page,
}) => {
  await installMockBridge(page);
  await openAgentsView(page);

  const agentPubkey = await addGenericAgent(page, "general", "Partial Bot");
  // null total + incomplete input → approx-partial display: row shows Partial
  // badge, bar shows ≈N* (approx-partial trailing), ingress trailing says "Partial".
  const partialBucket = {
    start: BASE,
    end: BASE + DAY,
    hasUnknownUsage: false,
    reportCount: 1,
    usage: {
      estimatedCostUsd: costField(null),
      inputTokens: usageField("800", true), // incomplete
      outputTokens: usageField("200", false),
      totalTokens: usageField(null),
      cacheReadTokens: usageField(null),
      cacheWriteTokens: usageField(null),
      freshInputTokens: usageField(null),
    },
  };
  await seedSeries(
    page,
    mockUsageSeries({
      agents: [
        mockAgentUsage(agentPubkey, {
          buckets: [partialBucket],
          usage: {
            estimatedCostUsd: costField(null),
            inputTokens: usageField("800", true), // incomplete
            outputTokens: usageField("200", false),
            totalTokens: usageField(null),
            cacheReadTokens: usageField(null),
            cacheWriteTokens: usageField(null),
            freshInputTokens: usageField(null),
          },
        }),
      ],
      buckets: [partialBucket],
    }),
  );

  const row = page.getByTestId(`agent-usage-row-${agentPubkey}`);
  await expect(row).toBeVisible();
  // Row: Partial badge present (dt.partial = true because inputTokens.incomplete).
  await expect(row.getByText("Partial", { exact: true })).toBeVisible();

  // Bar: approx-partial bucket renders ≈N* (the trailing * is the partial signal).
  const dailyBars = page.getByTestId("agent-usage-daily-bars");
  await expect(dailyBars).toBeVisible();
  await expect(dailyBars).toContainText("≈1K*");

  // Ingress: navigate to the Info tab and verify the trailing shows "Partial".
  await row.click();
  await expect(page.getByTestId("user-profile-panel")).toBeVisible();
  await page.getByTestId("user-profile-panel-back").click();
  await expect(page.getByTestId("user-profile-tab-info")).toBeVisible();
  const ingressRow = page.getByTestId(`user-profile-view-usage-${agentPubkey}`);
  await expect(ingressRow).toBeVisible();
  await expect(ingressRow).toContainText("Partial");
});

test("focused view renders the unknown-intervals and invalid-reports caveats under their independent gate conditions", async ({
  page,
}) => {
  await installMockBridge(page);
  await openAgentsView(page);

  const agentPubkey = await addGenericAgent(page, "general", "Coverage Bot");

  // State 1: incomplete I/O (inputTokens.incomplete=true) with invalidReportCount=0.
  // Only the unknown-intervals caveat must fire; invalid-reports must be absent.
  // This proves the unknown-intervals gate is I/O-incompleteness, not invalidReportCount.
  await test.step("incomplete I/O, no invalid reports → only unknown-intervals caveat", async () => {
    await seedSeries(
      page,
      mockUsageSeries({
        agents: [
          mockAgentUsage(agentPubkey, {
            hasUnknownUsage: true,
            reportCount: 2,
            usage: {
              estimatedCostUsd: costField(null),
              inputTokens: usageField("400", true), // incomplete → unknown-intervals fires
              outputTokens: usageField("100", false),
              totalTokens: usageField("500"),
              cacheReadTokens: usageField(null),
              cacheWriteTokens: usageField(null),
              freshInputTokens: usageField(null),
            },
          }),
        ],
        coverage: {
          firstArchivedAt: BASE,
          firstReportedAt: BASE,
          hasUnknownUsage: true,
          invalidReportCount: 0, // absent → invalid-reports must NOT fire
          lastArchivedAt: BASE + DAY,
          lastReportedAt: BASE + DAY,
          reportCount: 2,
        },
      }),
    );
    await page.getByTestId(`agent-usage-row-${agentPubkey}`).click();
    await expect(page.getByTestId("agent-usage-focused-view")).toBeVisible();
    await expect(page.getByTestId("agent-usage-focused-totals")).toBeVisible();
    await expect(
      page.getByTestId("agent-usage-focused-unknown-intervals-caveat"),
    ).toBeVisible();
    await expect(
      page.getByTestId("agent-usage-focused-invalid-reports-caveat"),
    ).toHaveCount(0);
  });

  // State 2: complete I/O (no incomplete flag) with invalidReportCount>0,
  // AND hasUnknownUsage=true (totalTokens incomplete) to decouple hasUnknownUsage
  // from I/O incompleteness. Per the gate comment in AgentUsageFocusedView.tsx,
  // hasUnknownUsage ORs total/cost incompleteness — which cannot prove an I/O
  // interval claim. A regression gating unknown-intervals on hasUnknownUsage
  // would wrongly fire here and fail the toHaveCount(0) assertion.
  // seedSeries navigates back to the agents overview, triggering a fresh query.
  await test.step("complete I/O, invalid reports present → only invalid-reports caveat", async () => {
    await seedSeries(
      page,
      mockUsageSeries({
        agents: [
          mockAgentUsage(agentPubkey, {
            hasUnknownUsage: true, // totalTokens incomplete → hasUnknownUsage true
            reportCount: 2,
            usage: {
              estimatedCostUsd: costField(null),
              inputTokens: usageField("400", false), // complete → unknown-intervals must NOT fire
              outputTokens: usageField("100", false),
              totalTokens: usageField("500", true), // incomplete total → hasUnknownUsage
              cacheReadTokens: usageField(null),
              cacheWriteTokens: usageField(null),
              freshInputTokens: usageField(null),
            },
          }),
        ],
        coverage: {
          firstArchivedAt: BASE,
          firstReportedAt: BASE,
          hasUnknownUsage: true, // mirrors agent-level: total incomplete, not I/O
          invalidReportCount: 1, // > 0 → invalid-reports fires
          lastArchivedAt: BASE + DAY,
          lastReportedAt: BASE + DAY,
          reportCount: 2,
        },
      }),
    );
    // The focused-view query has staleTime=60s — explicitly invalidate the React
    // Query cache so state 2 re-fetches from the updated mock rather than serving
    // state 1's cached response.
    await page.evaluate(() =>
      (
        window as Window & {
          __BUZZ_E2E_QUERY_CLIENT__?: {
            invalidateQueries: () => Promise<void>;
          };
        }
      ).__BUZZ_E2E_QUERY_CLIENT__?.invalidateQueries(),
    );
    await page.getByTestId(`agent-usage-row-${agentPubkey}`).click();
    await expect(page.getByTestId("agent-usage-focused-view")).toBeVisible();
    await expect(page.getByTestId("agent-usage-focused-totals")).toBeVisible();
    await expect(
      page.getByTestId("agent-usage-focused-invalid-reports-caveat"),
    ).toBeVisible();
    await expect(
      page.getByTestId("agent-usage-focused-unknown-intervals-caveat"),
    ).toHaveCount(0);
  });
});

test("the focused view shows the cache/fresh-input breakdown with exact totals and a per-model line when cache data is present", async ({
  page,
}) => {
  await installMockBridge(page);
  await openAgentsView(page);

  const agentPubkey = await addGenericAgent(page, "general", "Cache Bot");
  await seedSeries(
    page,
    mockUsageSeries({
      agents: [
        mockAgentUsage(agentPubkey, {
          models: [
            {
              harness: "claude-code",
              hasUnknownUsage: false,
              model: "claude-opus",
              reportCount: 1,
              usage: reportedUsage({
                totalTokens: "12000",
                inputTokens: "10000",
                outputTokens: "2000",
                cacheReadTokens: "6000",
                cacheWriteTokens: "1500",
                freshInputTokens: "2500",
              }),
            },
          ],
          usage: reportedUsage({
            inputTokens: "10000",
            outputTokens: "2000",
            totalTokens: "12000",
            cacheReadTokens: "6000",
            cacheWriteTokens: "1500",
            freshInputTokens: "2500",
          }),
        }),
      ],
    }),
  );

  await page.getByTestId(`agent-usage-row-${agentPubkey}`).click();
  await expect(page.getByTestId("agent-usage-focused-view")).toBeVisible();

  // Input-breakdown subsection: three exact stats, none rendered as zero.
  const cache = page.getByTestId("agent-usage-focused-cache");
  await expect(cache).toBeVisible();
  await expect(cache).toContainText("Cache read");
  await expect(
    page.getByTestId("agent-usage-focused-cache-read-value"),
  ).toHaveText("6,000");
  await expect(
    page.getByTestId("agent-usage-focused-cache-write-value"),
  ).toHaveText("1,500");
  await expect(
    page.getByTestId("agent-usage-focused-fresh-input-value"),
  ).toHaveText("2,500");
  // No Partial badge when every shown cache field is a complete value.
  await expect(cache.getByText("Partial", { exact: true })).toHaveCount(0);

  // Per-model breakdown line carries the same subsets, compact-formatted.
  const modelCache = page.getByTestId("agent-usage-model-cache-breakdown");
  await expect(modelCache).toContainText("Cache read 6K");
  await expect(modelCache).toContainText("Cache write 1.5K");
  await expect(modelCache).toContainText("Fresh 2.5K");
});

test("the focused view marks a known-but-incomplete cache field Partial, omits absent subsets, and hides the breakdown entirely when no cache data exists", async ({
  page,
}) => {
  await installMockBridge(page);
  await openAgentsView(page);

  const agentPubkey = await addGenericAgent(
    page,
    "general",
    "Partial Cache Bot",
  );

  await test.step("mixed cache completeness → Partial on the lower bound, absent fresh-input shows em-dash", async () => {
    await seedSeries(
      page,
      mockUsageSeries({
        agents: [
          mockAgentUsage(agentPubkey, {
            usage: {
              estimatedCostUsd: costField(null),
              inputTokens: usageField("10000"),
              outputTokens: usageField("2000"),
              totalTokens: usageField("12000"),
              // Known but incomplete (Will's mixed-window shape) → Partial.
              cacheReadTokens: usageField("6000", true),
              // Absent → must render "—", never 0.
              cacheWriteTokens: usageField(null),
              // Fail-closed: fresh input cannot be derived → unknown → "—".
              freshInputTokens: usageField(null, true),
            },
          }),
        ],
      }),
    );

    await page.getByTestId(`agent-usage-row-${agentPubkey}`).click();
    const cache = page.getByTestId("agent-usage-focused-cache");
    await expect(cache).toBeVisible();
    // Cache read: known lower bound → value plus Partial badge.
    await expect(
      page.getByTestId("agent-usage-focused-cache-read-value"),
    ).toHaveText("6,000");
    await expect(cache.getByText("Partial", { exact: true })).toBeVisible();
    // Cache write + fresh input are unknown → em-dash, never zero.
    await expect(
      page.getByTestId("agent-usage-focused-cache-write-value"),
    ).toHaveText("—");
    await expect(
      page.getByTestId("agent-usage-focused-fresh-input-value"),
    ).toHaveText("—");
  });

  await test.step("no cache data at all → the breakdown subsection is absent", async () => {
    await seedSeries(
      page,
      mockUsageSeries({
        agents: [
          mockAgentUsage(agentPubkey, {
            // Old-harness shape: i/o known, every cache subset absent.
            usage: reportedUsage({
              inputTokens: "10000",
              outputTokens: "2000",
              totalTokens: "12000",
            }),
          }),
        ],
      }),
    );
    await page.evaluate(() =>
      (
        window as Window & {
          __BUZZ_E2E_QUERY_CLIENT__?: {
            invalidateQueries: () => Promise<void>;
          };
        }
      ).__BUZZ_E2E_QUERY_CLIENT__?.invalidateQueries(),
    );
    await page.getByTestId(`agent-usage-row-${agentPubkey}`).click();
    await expect(page.getByTestId("agent-usage-focused-totals")).toBeVisible();
    await expect(page.getByTestId("agent-usage-focused-cache")).toHaveCount(0);
    await expect(
      page.getByTestId("agent-usage-model-cache-breakdown"),
    ).toHaveCount(0);
  });
});

test("overview and focused view distinguish an invalid-only window from ordinary empty windows", async ({
  page,
}) => {
  // An invalid-only window: invalidReportCount > 0 but agents[] and buckets[]
  // are empty (invalid rows are excluded from bucketing per A5/A11). The
  // overview must NOT say "No locally archived usage" (ordinary-absent text)
  // and the focused view must NOT show "outside-window" — both would mislabel
  // in-window-but-uncountable evidence as absent.
  await installMockBridge(page);
  await openAgentsView(page);

  const agentPubkey = await addGenericAgent(page, "general", "Invalid Bot");
  await seedSeries(
    page,
    mockUsageSeries({
      agents: [],
      buckets: [],
      coverage: {
        firstArchivedAt: BASE,
        firstReportedAt: null,
        hasUnknownUsage: true,
        invalidReportCount: 2, // the signal — no valid rows, but evidence exists
        lastArchivedAt: BASE + DAY,
        lastReportedAt: null,
        reportCount: 0,
      },
      hasArchivedEvidence: true, // A13: true for invalid-only windows too
    }),
  );

  // Overview: must reflect uncountable evidence, not ordinary absence.
  const empty = page.getByTestId("agent-usage-empty");
  await expect(empty).toBeVisible();
  await expect(empty).toContainText("could not be counted");
  await expect(empty).not.toContainText("No locally archived usage");

  // Focused view: invalid-only state, NOT outside-window.
  await navigateToProfile(page, agentPubkey, "usage");
  await expect(page.getByTestId("agent-usage-focused-view")).toBeVisible();
  await expect(
    page.getByTestId("agent-usage-focused-invalid-only"),
  ).toBeVisible();
  await expect(
    page.getByTestId("agent-usage-focused-outside-window"),
  ).toHaveCount(0);
});
