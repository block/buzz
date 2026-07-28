import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const SHOTS = "test-results/project-issue-assignee";
const OWNER = "deadbeef".repeat(8);
const REPO_ADDRESS = `30617:${OWNER}:buzz`;
const ISSUE_ID = "1".repeat(64);

test("project issues show signed assignee state", async ({ page }) => {
  await page.addInitScript(
    ({ assignee, issueId, owner, repoAddress }) => {
      const now = Math.floor(Date.now() / 1000);
      window.__BUZZ_E2E_EXTRA_PROJECT_EVENTS__ = [
        {
          id: issueId,
          kind: 1621,
          pubkey: assignee,
          created_at: now - 1,
          content: "Route this issue without implying execution.",
          tags: [
            ["a", repoAddress],
            ["subject", "Add signed issue routing"],
          ],
        },
        {
          id: "2".repeat(64),
          kind: 32001,
          pubkey: owner,
          created_at: now,
          content: "",
          tags: [
            ["d", issueId],
            ["e", issueId, "", "root"],
            ["p", assignee, "", "assignee"],
            ["a", repoAddress],
          ],
        },
      ];
    },
    {
      assignee: TEST_IDENTITIES.alice.pubkey,
      issueId: ISSUE_ID,
      owner: OWNER,
      repoAddress: REPO_ADDRESS,
    },
  );
  await installMockBridge(page);
  await page.setViewportSize({ width: 1280, height: 720 });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-projects-view").click();
  await page.getByRole("button", { name: "Issues", exact: true }).click();
  await page.getByRole("button", { name: "List layout" }).click();

  const issueRow = page
    .locator('[data-testid^="projects-issue-row-"]')
    .filter({ hasText: "Add signed issue routing" });
  await expect(issueRow).toContainText("assigned to alice");
  await waitForAnimations(page);
  await issueRow.screenshot({ path: `${SHOTS}/01-assigned-issue-row.png` });

  await issueRow
    .getByRole("button", { name: "View Add signed issue routing" })
    .click();
  const detail = page.getByTestId("project-issue-detail");
  await expect(detail).toContainText("Assignee");
  await expect(detail).toContainText("alice");
  await waitForAnimations(page);
  await detail.screenshot({ path: `${SHOTS}/02-assigned-issue-detail.png` });
});
