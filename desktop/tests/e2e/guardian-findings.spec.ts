import { expect, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const AGENT = TEST_IDENTITIES.tyler.pubkey;
const CHANNEL = "94a444a4-c0a3-5966-ab05-530c6ddc2301";

test("renders three privacy-projected Guardian alerts", async ({ page }) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: AGENT,
        name: "Observer Agent",
        status: "running",
        channelNames: ["agents"],
      },
    ],
    numbatFindingBatch: {
      nextOffset: 512,
      reset: false,
      rejectedRecords: 0,
      health: {
        state: "active",
        detail: "Guardian callback execution is verified.",
      },
      findings: [
        {
          findingId: "finding-low",
          ruleId: "source.git_remote_tamper",
          title: "Git remote-routing change requested",
          severity: "low",
          detectedAt: "2026-07-31T18:00:00Z",
          sourceAgent: AGENT,
          sessionId: "session-guardian",
          channelId: CHANNEL,
          turnId: "turn-guardian",
          evidenceCount: 1,
        },
        {
          findingId: "finding-medium",
          ruleId: "secrets.agent_read_env",
          title: "Sensitive environment data accessed",
          severity: "medium",
          detectedAt: "2026-07-31T18:00:01Z",
          sourceAgent: AGENT,
          sessionId: "session-guardian",
          channelId: CHANNEL,
          turnId: "turn-guardian",
          evidenceCount: 1,
        },
        {
          findingId: "finding-high",
          ruleId: "chain.secret_read_then_egress",
          title: "Possible secret exfiltration",
          severity: "high",
          detectedAt: "2026-07-31T18:00:02Z",
          sourceAgent: AGENT,
          sessionId: "session-guardian",
          channelId: CHANNEL,
          turnId: "turn-guardian",
          evidenceCount: 2,
        },
      ],
    },
  });

  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_SEED_OBSERVER_EVENTS__ === "function",
  );
  await page.getByTestId("channel-agents").click();

  const messageRow = page
    .getByTestId("message-row")
    .filter({ has: page.getByText("Observer Agent", { exact: false }) })
    .first();
  await expect(messageRow).toBeVisible();
  await messageRow.getByRole("button").first().click();
  await page.getByTestId(`user-profile-view-activity-${AGENT}`).click();

  const guardian = page.getByTestId("guardian-security-findings");
  await expect(guardian).toBeVisible();
  await expect(guardian).toContainText("active");
  await expect(guardian.locator("article")).toHaveCount(3);
  await expect(guardian).toContainText("LOW");
  await expect(guardian).toContainText("MEDIUM");
  await expect(guardian).toContainText("HIGH");
  await expect(guardian).not.toContainText("observed_command");
  await expect(guardian).not.toContainText("/private/");
});
