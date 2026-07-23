import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const SHOTS = "test-results/agent-access-warning";

async function openAgentAccessDialog(
  page: import("@playwright/test").Page,
  agentPubkey: string,
) {
  if (!(await page.getByTestId("members-sidebar").isVisible())) {
    await page.getByTestId("channel-general").click();
    await page.getByTestId("channel-members-trigger").click();
    await expect(page.getByTestId("members-sidebar")).toBeVisible();
  }

  const row = page.getByTestId(`sidebar-member-${agentPubkey}`);
  const menu = page.getByTestId(`sidebar-member-menu-${agentPubkey}`);
  await row.hover();
  await menu.focus();
  await menu.press("Enter");
  await page.getByTestId(`sidebar-edit-respond-to-${agentPubkey}`).click();

  await expect(
    page.getByRole("dialog", { name: "Manage agent access" }),
  ).toBeVisible();
}

test("open agent access explains the available access before save", async ({
  page,
}) => {
  const agent = TEST_IDENTITIES.charlie;
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: agent.pubkey,
        name: "Hack Day Helper",
        status: "running",
        channelNames: ["general"],
        respondTo: "owner-only",
      },
    ],
  });
  await page.goto("/");
  await openAgentAccessDialog(page, agent.pubkey);

  const accessSelect = page.getByTestId("agent-respond-to-select");
  await expect(accessSelect).toHaveValue("owner-only");
  await expect(page.getByTestId("agent-access-warning")).toHaveCount(0);
  const saveAccess = page.getByRole("button", { name: "Save access" });
  await expect(saveAccess).toBeVisible();

  const commandsBeforeSave = await page.evaluate(
    () => window.__BUZZ_E2E_COMMAND_LOG__?.length ?? 0,
  );
  await accessSelect.selectOption("anyone");
  const warning = page.getByTestId("agent-access-warning");
  await expect(warning).toBeVisible();
  await expect(warning).toContainText(
    "Anyone can send instructions to this agent.",
  );
  await expect(warning).toContainText(
    "It may use files, accounts, and tools it can access on the computer or server where it runs.",
  );

  await waitForAnimations(page);
  await page
    .getByRole("dialog", { name: "Manage agent access" })
    .screenshot({ path: `${SHOTS}/open-access-warning.png` });

  await saveAccess.click();
  await expect(
    page.getByRole("dialog", { name: "Manage agent access" }),
  ).not.toBeVisible();
  await expect
    .poll(async () =>
      page.evaluate((start) => {
        const commands = window.__BUZZ_E2E_COMMAND_LOG__ ?? [];
        return commands
          .slice(start)
          .some(
            (entry) =>
              entry.command === "update_managed_agent" &&
              (entry.payload as { input?: { respondTo?: string } })?.input
                ?.respondTo === "anyone",
          );
      }, commandsBeforeSave),
    )
    .toBe(true);

  await openAgentAccessDialog(page, agent.pubkey);
  await expect(accessSelect).toHaveValue("anyone");
  await accessSelect.selectOption("allowlist");
  await expect(warning).toHaveCount(0);
  await expect(
    page
      .getByTestId("agent-respond-to-allowlist")
      .getByText("Selected people", { exact: true }),
  ).toBeVisible();
});
