import { expect, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const AGENT_PUBKEY = TEST_IDENTITIES.tyler.pubkey;
const AGENT_NAME = "Patina Agent";

async function openRuntimePanel(page: import("@playwright/test").Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("channel-agents").click();
  await expect(page.getByTestId("chat-title")).toHaveText("agents");

  const messageRow = page.getByTestId("message-row").filter({
    has: page.getByText(AGENT_NAME, { exact: false }),
  });
  await expect(messageRow.first()).toBeVisible({ timeout: 5_000 });
  await messageRow.first().getByRole("button").first().click();

  const panel = page.getByTestId("user-profile-panel");
  await expect(panel).toBeVisible({ timeout: 10_000 });
  await panel.getByRole("tab", { name: "Runtime" }).click();
  await expect(panel.getByText("MCP Servers", { exact: true })).toBeVisible({
    timeout: 10_000,
  });
  return panel;
}

test.describe("Patina remote MCP", () => {
  test("connects, clears the key, toggles, tests, and disconnects", async ({
    page,
  }) => {
    await installMockBridge(page, {
      managedAgents: [
        {
          channelNames: ["agents"],
          name: AGENT_NAME,
          pubkey: AGENT_PUBKEY,
          status: "running",
        },
      ],
    });

    const panel = await openRuntimePanel(page);
    await panel.getByTestId("connect-patina").click();
    await panel.getByTestId("patina-workspace-slug").fill("Acme-Team");
    await panel.getByTestId("patina-api-key").fill("pk_viewer_secret");
    await panel.getByTestId("patina-test-connect").click();

    const connection = panel.getByTestId("patina-connection");
    await expect(connection).toContainText("Patina · Patina Demo");
    await expect(connection).toContainText("Buzz Viewer · connected");

    await connection.getByRole("button", { name: "Reconnect" }).click();
    await expect(panel.getByTestId("patina-workspace-slug")).toHaveValue(
      "acme-team",
    );
    await expect(panel.getByTestId("patina-api-key")).toHaveValue("");
    await panel.getByRole("button", { name: "Cancel" }).click();

    await connection.getByRole("button", { name: "Test" }).click();
    await expect(connection).toContainText("connected");
    await connection.getByRole("button", { name: "Disable" }).click();
    await expect(connection).toContainText("disabled");
    await connection.getByRole("button", { name: "Enable" }).click();
    await expect(connection).toContainText("connected");

    await connection.screenshot({
      path: "test-results/patina-mcp/connected.png",
    });

    await connection.getByRole("button", { name: "Disconnect" }).click();
    await expect(connection).not.toBeVisible();
    await expect(panel.getByTestId("connect-patina")).toBeVisible();
  });
});
