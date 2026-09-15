import { expect, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

async function waitForMockLiveSubscription(
  page: import("@playwright/test").Page,
  channelName: string,
) {
  await expect
    .poll(() =>
      page.evaluate(
        (name) =>
          window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
            channelName: name,
          }) ?? false,
        channelName,
      ),
    )
    .toBe(true);
}

// Regression: the forum list read a single page and dropped the `before`
// cursor the relay returns, so a forum past the page size had no way to reach
// its older posts.
test("a forum past one page loads older posts on demand", async ({ page }) => {
  await installMockBridge(page);
  await page.goto("/");

  await page.getByTestId("channel-general").click();
  await waitForMockLiveSubscription(page, "watercooler");
  // Two mock posts already exist, so 49 more crosses the 50-post page size.
  await page.evaluate(
    ({ count, pubkey }) => {
      for (let index = 1; index <= count; index += 1) {
        window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
          channelName: "watercooler",
          content: `Paged post ${index}`,
          kind: 45001,
          pubkey,
        });
      }
    },
    { count: 49, pubkey: TEST_IDENTITIES.alice.pubkey },
  );

  await page.getByTestId("channel-watercooler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("watercooler");

  const loadOlder = page.getByRole("button", { name: "Load older posts" });
  await expect(loadOlder).toBeVisible();

  await loadOlder.click();
  // The second page is short, which ends the list — the control must not
  // offer a page that does not exist.
  await expect(loadOlder).toHaveCount(0);
  await expect(
    page.getByText("Release checklist: async feedback thread."),
  ).toBeVisible();
});
