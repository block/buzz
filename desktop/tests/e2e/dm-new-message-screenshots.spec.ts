import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import {
  installMockBridge,
  openNewMessagePage,
  TEST_IDENTITIES,
} from "../helpers/bridge";

const SHOTS = "test-results/dm-new-message";

test("captures the new-message loading skeleton", async ({ page }) => {
  await installMockBridge(page, {
    managedAgents: [],
    relayAgents: [],
    userSearchDelayMs: 10_000,
  });
  await page.goto("/");
  await openNewMessagePage(page);

  const search = page.getByTestId("new-dm-search");
  await search.fill("Alex");
  await expect(search).toHaveValue("Alex");
  await expect(page.getByTestId("new-dm-loading")).toBeVisible();
  await expect(
    page.locator("[data-testid^='new-dm-result-']:visible"),
  ).toHaveCount(0);

  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/01-search-loading.png` });
});

test("captures selected recipients with the picker open", async ({ page }) => {
  await installMockBridge(page);
  await page.goto("/");
  await openNewMessagePage(page);

  for (const identity of [TEST_IDENTITIES.charlie, TEST_IDENTITIES.bob]) {
    await page.getByTestId("new-dm-search").fill(identity.username);
    await page.getByTestId(`new-dm-result-${identity.pubkey}`).click();
  }

  await expect(
    page.locator("button[data-testid^='new-dm-selected-']"),
  ).toHaveCount(2);
  const search = page.getByTestId("new-dm-search");
  await search.click();
  await expect(page.getByTestId("new-message-recipient-popover")).toBeVisible();

  await waitForAnimations(page);
  await expect(page.getByTestId("new-message-recipient-popover")).toBeVisible();
  await page.screenshot({ path: `${SHOTS}/02-selected-recipients.png` });
});

test("captures the enabled first-message composer", async ({ page }) => {
  await installMockBridge(page);
  await page.goto("/");
  await openNewMessagePage(page);

  await page.getByTestId("new-dm-search").fill("charlie");
  await page
    .getByTestId(`new-dm-result-${TEST_IDENTITIES.charlie.pubkey}`)
    .click();
  await page.getByTestId("new-dm-search").press("Escape");
  await expect(page.getByTestId("new-message-recipient-popover")).toBeHidden();
  await page
    .getByTestId("message-input")
    .fill("Hey Charlie — want to start a new thread?");
  await expect(page.getByTestId("send-message")).toBeEnabled();

  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/03-first-message-compose.png` });
});
