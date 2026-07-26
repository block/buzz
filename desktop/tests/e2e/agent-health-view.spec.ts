import { expect, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

const SHOTS = "test-results/agent-health-view";
const AGENT = {
  pubkey: TEST_IDENTITIES.outsider.pubkey,
  name: "Hermes",
  avatarUrl: "https://api.dicebear.com/9.x/initials/svg?seed=Hermes",
  agentCommand: "codex-acp",
  systemPrompt: "Own engineering and infrastructure work.",
  model: "gpt-5",
  provider: "openai",
  status: "running" as const,
  respondTo: "owner-only" as const,
};

async function openHealthCard(
  page: import("@playwright/test").Page,
  viewport: { width: number; height: number },
) {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-agents-view").click();
  const agentButton = page.getByRole("button", {
    name: `${AGENT.name} agent profile`,
  });
  await expect(agentButton).toBeVisible({ timeout: 10_000 });
  await agentButton.click();
  const panel = page.getByTestId("user-profile-panel");
  await expect(panel).toBeVisible();
  await panel.getByRole("tab", { name: "Runtime" }).click();
  await expect(panel.getByTestId("agent-health-card")).toBeVisible();
  await page.setViewportSize(viewport);
  return panel;
}

test.describe("agent health view", () => {
  test("390px loading and empty memberships remain explicit", async ({
    page,
  }) => {
    await installMockBridge(page, {
      agentListDelayMs: 150,
      channelsReadDelayMs: 1_500,
      managedAgents: [{ ...AGENT, model: null, provider: null }],
    });

    const panel = await openHealthCard(page, { width: 390, height: 844 });
    await expect(panel.getByTestId("agent-health-channels")).toContainText(
      "Loading",
    );
    await expect(panel.getByTestId("agent-health-model")).toHaveAttribute(
      "data-availability",
      "unknown",
    );
    await waitForAnimations(page);
    await panel.screenshot({ path: `${SHOTS}/390-loading-partial.png` });

    await expect(panel.getByTestId("agent-health-channels")).toContainText(
      "None",
      { timeout: 5_000 },
    );
    await waitForAnimations(page);
    await panel.screenshot({ path: `${SHOTS}/390-empty.png` });
  });

  test("768px channel error is unavailable, not empty", async ({ page }) => {
    await installMockBridge(page, {
      channelsReadError: "membership query unavailable",
      managedAgents: [AGENT],
    });

    const panel = await openHealthCard(page, { width: 768, height: 900 });
    const channels = panel.getByTestId("agent-health-channels");
    await expect(channels).toHaveAttribute("data-availability", "unavailable");
    await expect(channels).toContainText("Buzz could not load");
    await waitForAnimations(page);
    await panel.screenshot({ path: `${SHOTS}/768-error.png` });
  });

  test("1280px populated state and warnings render together", async ({
    page,
  }) => {
    await installMockBridge(page, {
      managedAgents: [
        {
          ...AGENT,
          channelNames: ["engineering", "agents"],
          needsRestart: true,
          lastError: "Provider authentication needs attention",
        },
      ],
    });

    const panel = await openHealthCard(page, { width: 1280, height: 900 });
    await expect(panel.getByTestId("agent-health-provider")).toContainText(
      "openai",
    );
    await expect(panel.getByTestId("agent-health-model")).toContainText(
      "gpt-5",
    );
    await expect(panel.getByTestId("agent-health-channels")).toContainText(
      "#engineering",
    );
    await expect(panel.getByTestId("agent-health-warnings")).toContainText(
      "Restart required",
    );
    await expect(
      panel.getByTestId("agent-health-last-successful-mention"),
    ).toHaveAttribute("data-availability", "unavailable");
    await waitForAnimations(page);
    await panel.screenshot({ path: `${SHOTS}/1280-populated-warnings.png` });
  });
});
