import { expect, test, bootstrapE2ePage } from "../helpers/test";

import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

test("captures global and current-channel search scope states", async ({
  page,
}) => {
  await installMockBridge(page);
  await bootstrapE2ePage(page, "/");
  await page.getByTestId("channel-deep-history").click();
  await expect(page.getByTestId("chat-title")).toHaveText("deep-history");
  await expect(
    page.locator('[data-message-id^="mock-deep-history-"]').first(),
  ).toBeVisible();
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(page).toHaveURL(
    /#\/channels\/9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50$/,
  );

  await page.keyboard.press("ControlOrMeta+k");
  const searchDialog = page.getByTestId("search-results");
  const scopeAction = page.getByTestId("search-current-channel-control");
  await expect(scopeAction).toContainText("Search in");
  await expect(scopeAction).toContainText("#general");
  await waitForAnimations(page);
  await searchDialog.screenshot({
    path: "test-results/search-scope/01-global-channel-scope-action.png",
  });

  await scopeAction.click();
  await page.getByTestId("search-dialog-input").fill("welcome");
  await expect(page.getByTestId("search-channel-scope-chip")).toHaveText(
    /#general/,
  );
  await expect(searchDialog).toContainText("Welcome to #general");
  await waitForAnimations(page);
  await searchDialog.screenshot({
    path: "test-results/search-scope/02-current-channel-scoped.png",
  });

  await page.getByTestId("search-channel-scope-chip").click();
  await page.getByTestId("search-dialog-input").fill("deep history message");
  await expect(
    page.locator('[data-search-section="messages"] .search-result-row'),
  ).toHaveCount(40);
  await page.getByTestId("search-results-list").evaluate((element) => {
    element.scrollTop = 180;
  });
  await expect(scopeAction).not.toBeInViewport();
  await waitForAnimations(page);
  await searchDialog.screenshot({
    path: "test-results/search-scope/03-expanded-scrollable-results.png",
  });

  await page.keyboard.press("Escape");
  await page.getByTestId("channel-alice-tyler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("alice-tyler");
  await expect(page).toHaveURL(
    /#\/channels\/f48efb06-0c93-5025-aac9-2e646bb6bfa8$/,
  );
  await page.keyboard.press("ControlOrMeta+k");
  await expect(scopeAction).toContainText("Search conversation with alice");
  await expect(scopeAction).toContainText(
    "Search messages in this conversation.",
  );
  await waitForAnimations(page);
  await searchDialog.screenshot({
    path: "test-results/search-scope/04-dm-conversation-scope-action.png",
  });
});
