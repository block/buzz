import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

const AGENT_PUBKEY = "a".repeat(64);

test("tagged agent slash menu filters and inserts a runtime command", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: AGENT_PUBKEY,
        name: "alice",
        status: "running",
        channelNames: ["general"],
      },
    ],
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("channel-general").click();

  const input = page.getByTestId("message-input");
  await input.fill("@ali");
  await page
    .getByTestId("mention-autocomplete")
    .getByRole("button", { name: /alice/i })
    .click();

  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_SEED_OBSERVER_EVENTS__ === "function",
  );
  await page.evaluate((agentPubkey) => {
    window.__BUZZ_E2E_SEED_OBSERVER_EVENTS__?.({
      agentPubkey,
      events: [
        {
          seq: 1,
          timestamp: "2026-08-02T00:00:00Z",
          kind: "acp_read",
          agentIndex: 0,
          channelId: "94a444a4-c0a3-5966-ab05-530c6ddc2301",
          sessionId: "session-slash-menu",
          turnId: "turn-slash-menu",
          payload: {
            method: "session/update",
            params: {
              update: {
                sessionUpdate: "available_commands_update",
                availableCommands: [
                  {
                    name: "ad-monitor",
                    description: "Review the latest advertising performance",
                  },
                  {
                    name: "creative-run",
                    description: "Plan the next creative production batch",
                  },
                  {
                    name: "cfo",
                    description: "Run the financial health check",
                  },
                ],
              },
            },
          },
        },
      ],
    });
  }, AGENT_PUBKEY);

  await input.pressSequentially("/");
  const menu = page.getByTestId("slash-command-autocomplete");
  await expect(menu).toBeVisible();
  await expect(menu.getByText("/ad-monitor", { exact: true })).toBeVisible();
  await expect(menu.getByText("/creative-run", { exact: true })).toBeVisible();

  await input.pressSequentially("ad");
  await expect(menu.getByText("/ad-monitor", { exact: true })).toBeVisible();
  await expect(menu.getByText("/creative-run", { exact: true })).toHaveCount(0);

  await waitForAnimations(page);
  await menu.screenshot({
    path: "test-results/slash-command-picker/filtered-menu.png",
  });

  await input.focus();
  await page.keyboard.press("Enter");
  await expect(input).toHaveText("@alice /ad-monitor ");
  await expect(menu).toHaveCount(0);
});
