import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const DEFAULT_MOCK_PUBKEY = "deadbeef".repeat(8);
const BUZZ_REPO_ADDRESS = `30617:${DEFAULT_MOCK_PUBKEY}:buzz`;

test("Buzz Git pull request renders and stays actionable in Inbox", async ({
  page,
}) => {
  await page.addInitScript(() => {
    window.localStorage.setItem(
      "buzz-feature-overrides-v1",
      JSON.stringify({ projects: true }),
    );
  });
  await installMockBridge(page);
  await page.setViewportSize({ width: 1024, height: 720 });

  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-projects-view").click();
  await page.getByRole("button", { name: "Repositories", exact: true }).click();
  await page
    .locator(
      '[data-testid="project-card-buzz"], [data-testid="project-row-buzz"]',
    )
    .first()
    .click();
  await page.getByRole("tab", { name: "Pull Request" }).click();

  const alicePullRequest = page
    .getByTestId("project-pull-request-row")
    .filter({ hasText: "alice" })
    .first();
  await expect(alicePullRequest).toBeVisible({ timeout: 10_000 });
  const pullRequestId = await alicePullRequest.getAttribute(
    "data-project-event-id",
  );
  expect(pullRequestId).toBeTruthy();

  await page.getByRole("button", { name: "Inbox", exact: true }).click();
  await page.evaluate(
    ({ author, id, repoAddress, viewer }) => {
      window.__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__?.({
        id,
        kind: 1618,
        pubkey: author,
        content: "Inbox rendering verification",
        created_at: Math.floor(Date.now() / 1000) + 1,
        channel_id: null,
        channel_name: "",
        channel_type: null,
        tags: [
          ["a", repoAddress],
          ["p", viewer],
          ["subject", "Inbox rendering verification"],
        ],
        category: "mention",
      });
    },
    {
      author: TEST_IDENTITIES.alice.pubkey,
      id: pullRequestId as string,
      repoAddress: BUZZ_REPO_ADDRESS,
      viewer: DEFAULT_MOCK_PUBKEY,
    },
  );

  const inboxRow = page.getByTestId(`home-inbox-item-${pullRequestId}`);
  await expect(inboxRow).toBeVisible({ timeout: 10_000 });
  await inboxRow.locator(":scope > div").first().click();

  const detail = page.getByTestId("home-project-inbox-detail");
  const card = page.getByTestId("project-inbox-work-item-card");
  const layout = page.getByTestId("project-inbox-work-item-layout");
  await expect(detail).toBeVisible();
  await expect(card).toBeVisible();
  await expect(
    detail.locator('[data-testid^="project-inbox-author-avatar-"]'),
  ).toBeVisible();
  await expect(
    detail.getByRole("heading", {
      name: "alice sent you a pull request",
      exact: true,
    }),
  ).toBeVisible();
  await expect(page.getByText("Inbox rendering verification")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Approve", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Request changes", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Merge", exact: true }),
  ).toBeVisible();

  const columnCount = await layout.evaluate(
    (element) =>
      getComputedStyle(element).gridTemplateColumns.split(" ").filter(Boolean)
        .length,
  );
  expect(columnCount).toBe(1);

  await waitForAnimations(page);
  await detail.screenshot({
    path: "test-results/project-inbox/01-pull-request-detail.png",
  });
});
