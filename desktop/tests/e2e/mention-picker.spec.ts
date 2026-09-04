import { expect, test, type Page } from "@playwright/test";
import { installMockBridge, openCreateChannelDialog } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

const OWNER = "deadbeef".repeat(8);
const A = "11".repeat(32);
const parent = process.env.MENTION_PARENT === "1";
async function capture(page: Page, name: string) {
  await waitForAnimations(page);
  await page.screenshot({
    path: `test-results/mention-picker/${parent ? "before" : "after"}-${name}.png`,
    clip: { x: 256, y: 380, width: 1024, height: 520 },
  });
}
test.use({ viewport: { width: 1280, height: 900 } });

test("fresh create and Add member then first @ uses governing directory refresh", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [],
    relayAgents: [
      {
        pubkey: A,
        name: "Fresh Scout",
        ownerPubkey: OWNER,
        respondTo: "anyone",
      },
    ],
    searchProfiles: [
      {
        pubkey: A,
        displayName: "Fresh Scout",
        isAgent: true,
        ownerPubkey: OWNER,
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  // A successful warm-but-old directory is fixture input, not a manual repair.
  await page.evaluate(() => {
    const client = window.__BUZZ_E2E_QUERY_CLIENT__ as unknown as {
      setQueryData: (key: string[], data: unknown) => void;
    };
    client.setQueryData(["relay-agents"], []);
  });
  await openCreateChannelDialog(page);
  await page.getByTestId("create-channel-name").fill("fresh-picker-fixture");
  await page.getByTestId("create-channel-submit").click();
  await expect(page.getByTestId("chat-title")).toHaveText(
    "fresh-picker-fixture",
  );
  await page.getByTestId("channel-members-trigger").click();
  await page.getByTestId("channel-management-search-users").fill("Fresh Scout");
  await page.getByTestId(`channel-user-search-result-${A}`).click();
  await expect(page.getByTestId(`sidebar-member-${A}`)).toBeVisible();
  await page
    .getByRole("dialog", { name: "Channel members" })
    .getByRole("button", { name: "Close", exact: true })
    .click();
  await page.getByTestId("message-input").fill("@Fresh");
  const row = page.getByTestId(`mention-suggestion-${A}`);
  await expect(row).toBeVisible();
  await expect(row).not.toContainText(/not in channel/i);
  // Same settled camera on the unchanged parent, even if no row is admitted.
  await page.waitForTimeout(400);
  await capture(page, "fresh-add");
});
