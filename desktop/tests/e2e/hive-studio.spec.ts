import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

test.beforeEach(async ({ page }) => {
  await page.route("**/flow-studio/blocks", async (route) => {
    await route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({
        blocks: [
          {
            block_type: "http",
            label: "HTTP Request",
            category: "http",
            description: "Call an external URL",
          },
        ],
      }),
    });
  });

  await page.route("**/agent-studio/graph", async (route) => {
    await route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({
        nodes: [{ id: "agent:scout", kind: "agent", slug: "scout" }],
        edges: [],
      }),
    });
  });

  await page.route("**/agent-studio/costs", async (route) => {
    await route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({
        total_cost_usd: 0.042,
        acp_session_cost_usd: 0.041,
        flow_block_cost_usd: 0.001,
        total_tokens: 1200,
        session_count: 1,
        sessions: [],
      }),
    });
  });

  await installMockBridge(page);
});

test("navigates to Flow Studio and shows canvas shell", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("open-flow-studio-view").click();
  await expect(page).toHaveURL(/#\/flow-studio$/);
  await expect(page.getByTestId("flow-studio-view")).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Flow Studio" }),
  ).toBeVisible();
  await expect(page.getByText("HTTP Request")).toBeVisible();
});

test("navigates to Agent Studio and shows graph shell", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("open-agent-studio-view").click();
  await expect(page).toHaveURL(/#\/agent-studio$/);
  await expect(page.getByTestId("agent-studio-view")).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Agent Studio" }),
  ).toBeVisible();
  await expect(page.getByText("scout")).toBeVisible();
});
