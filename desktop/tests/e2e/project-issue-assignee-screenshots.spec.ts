import { expect, test } from "@playwright/test";
import { finalizeEvent } from "nostr-tools/pure";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const SHOTS = "test-results/project-issue-assignee";
const OWNER = "deadbeef".repeat(8);
const REPO_ADDRESS = `30617:${OWNER}:buzz`;

test("project issues show signed assignee state", async ({ page }) => {
  const secretKey = Uint8Array.from(
    TEST_IDENTITIES.alice.privateKey
      .match(/../g)
      ?.map((byte) => Number.parseInt(byte, 16)) ?? [],
  );
  const now = Math.floor(Date.now() / 1000);
  const issue = finalizeEvent(
    {
      kind: 1621,
      created_at: now - 1,
      content: "Route this issue without implying execution.",
      tags: [
        ["a", REPO_ADDRESS],
        ["subject", "Add signed issue routing"],
      ],
    },
    secretKey,
  );
  const assignment = finalizeEvent(
    {
      kind: 32001,
      created_at: now,
      content: "",
      tags: [
        ["d", issue.id],
        ["e", issue.id, "", "root"],
        ["p", TEST_IDENTITIES.alice.pubkey, "", "assignee"],
        ["a", REPO_ADDRESS],
      ],
    },
    secretKey,
  );
  await page.addInitScript(
    ({ events }) => {
      window.__BUZZ_E2E_EXTRA_PROJECT_EVENTS__ = events;
    },
    {
      events: [issue, assignment],
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
