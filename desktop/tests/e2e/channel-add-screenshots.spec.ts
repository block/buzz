import { test } from "@playwright/test";

import { installMockBridge, openChannelBrowser } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

const OUTDIR = "test-results/channel-add";

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

test("capture: add-channel default state", async ({ page }) => {
  await page.goto("/");
  await openChannelBrowser(page);
  await page.getByTestId("channel-browser-dialog").waitFor();
  await waitForAnimations(page);
  await page.screenshot({ path: `${OUTDIR}/01-add-channel-default.png` });
});

test("capture: create row on partial match", async ({ page }) => {
  await page.goto("/");
  await openChannelBrowser(page);
  await page.getByTestId("channel-browser-search").fill("desig");
  await page.getByTestId("channel-browser-create-row").waitFor();
  await waitForAnimations(page);
  await page.screenshot({ path: `${OUTDIR}/02-create-row-partial.png` });
});

test("capture: create row on no match", async ({ page }) => {
  await page.goto("/");
  await openChannelBrowser(page);
  await page.getByTestId("channel-browser-search").fill("release-notes");
  await page.getByTestId("channel-browser-create-row").waitFor();
  await waitForAnimations(page);
  await page.screenshot({ path: `${OUTDIR}/03-create-row-no-match.png` });
});

test("capture: prefilled create form", async ({ page }) => {
  await page.goto("/");
  await openChannelBrowser(page);
  await page.getByTestId("channel-browser-search").fill("release-notes");
  await page.getByTestId("channel-browser-create-row").click();
  await page.getByTestId("create-channel-name").waitFor();
  await waitForAnimations(page);
  await page.screenshot({ path: `${OUTDIR}/04-create-form.png` });
});
