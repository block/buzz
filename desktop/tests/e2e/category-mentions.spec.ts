import { expect, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

const SHOTS = "test-results/category-mentions";

test.use({ viewport: { width: 1280, height: 720 } });

test("@agents unfurls into every agent in the channel", async ({ page }) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: TEST_IDENTITIES.charlie.pubkey,
        name: "charlie",
        status: "running",
        channelNames: ["general"],
      },
      {
        pubkey: "8".repeat(64),
        name: "Scout",
        status: "running",
        channelNames: ["general"],
      },
    ],
  });

  await page.goto("/");
  await page.getByTestId("channel-general").click();

  const composer = page.getByTestId("message-composer");
  const input = composer.getByTestId("message-input");
  await input.fill("Status check @agent");

  const autocomplete = composer.getByTestId("mention-autocomplete");
  const categoryRow = autocomplete.getByTestId(
    "mention-suggestion-category-agents",
  );
  await expect(categoryRow).toContainText("agents");
  await expect(categoryRow).toContainText("category · 2 agents");

  await waitForAnimations(page);
  await page.screenshot({
    path: `${SHOTS}/01-agents-category-suggestion.png`,
    clip: { x: 240, y: 380, width: 800, height: 320 },
  });

  await input.press("Enter");
  await expect
    .poll(() => input.evaluate((element) => element.textContent))
    .toContain("Status check @charlie @Scout");
  await expect(input.locator(".agent-mention-highlight")).toHaveCount(2);

  await waitForAnimations(page);
  await page.screenshot({
    path: `${SHOTS}/02-unfurled-agent-members.png`,
    clip: { x: 240, y: 480, width: 800, height: 220 },
  });
});

test("@people unfurls into human members and excludes the sender", async ({
  page,
}) => {
  await installMockBridge(page);

  await page.goto("/");
  await page.getByTestId("channel-general").click();

  const composer = page.getByTestId("message-composer");
  const input = composer.getByTestId("message-input");
  await input.fill("Heads up @peopl");

  const autocomplete = composer.getByTestId("mention-autocomplete");
  const categoryRow = autocomplete.getByTestId(
    "mention-suggestion-category-people",
  );
  await expect(categoryRow).toContainText("people");
  await expect(categoryRow).toContainText("people in this channel");

  await input.press("Enter");
  await expect
    .poll(() => input.evaluate((element) => element.textContent))
    .toContain("Heads up @bob");

  const text = await input.evaluate((element) => element.textContent);
  expect(text).not.toContain("@agents");
  // The sender never appears in their own @people unfurl.
  expect(text?.match(/@/g)?.length).toBe(1);
});
