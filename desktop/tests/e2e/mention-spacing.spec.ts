import { expect, test } from "@playwright/test";
import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

// An ordinary existing channel member: no discovery, invitation or runtime needed.
test.beforeEach(async ({ page }) => {
  await installMockBridge(page, {
    searchProfiles: [
      { pubkey: TEST_IDENTITIES.alice.pubkey, displayName: "Alice Chen" },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
});

for (const deliberateMove of [false, true]) {
  test(`multi-word mention ${deliberateMove ? "respects intentional ArrowLeft" : "preserves separator when typing immediately"}`, async ({
    page,
  }) => {
    const input = page.getByTestId("message-input");
    await input.fill("Hey @Ali");
    await page
      .getByTestId("message-composer")
      .getByTestId("mention-autocomplete")
      .getByText("Alice Chen", { exact: true })
      .click();
    if (deliberateMove) await page.keyboard.press("ArrowLeft");
    await page.keyboard.type("hello");
    expect((await input.innerText()).trimEnd()).toBe(
      deliberateMove ? "Hey @Alice Chenhello" : "Hey @Alice Chen hello",
    );
    await waitForAnimations(page);
    await page.getByTestId("message-composer").screenshot({
      path: `test-results/mention-spacing/${deliberateMove ? "intentional-caret" : "separator"}.png`,
    });
  });
}
