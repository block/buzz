import { expect, test, type Page } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const MIRA_PUBKEY =
  "8f83d6b7f3d74f7d933ae3a54dd8c6cc85c7f98e531c16e5a827b953441a8d67";

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
});

async function lastChannelMentionTags(page: Page) {
  return page.evaluate(() => {
    const entries =
      (
        window as Window & {
          __BUZZ_E2E_COMMAND_PAYLOADS__?: Array<{
            command: string;
            payload: { mentionTags?: string[][] | null };
          }>;
        }
      ).__BUZZ_E2E_COMMAND_PAYLOADS__ ?? [];
    return (
      entries.findLast((entry) => entry.command === "send_channel_message")
        ?.payload.mentionTags ?? null
    );
  });
}

test("@everyone autocomplete explains and sends an exact standard p-tag snapshot", async ({
  page,
}) => {
  const input = page.getByTestId("message-input");
  await input.fill("Ship it @ev");

  const suggestion = page.getByTestId("mention-suggestion-audience-everyone");
  await expect(suggestion).toContainText("everyone");
  await expect(suggestion).toContainText("Notify everyone in this channel");
  await suggestion.click();
  await expect(input).toHaveText("Ship it @everyone ");

  await page.getByTestId("send-message").click();
  await expect(input).toBeEmpty();

  await expect
    .poll(() => lastChannelMentionTags(page))
    .toEqual([
      ["buzz-audience-ref", "everyone"],
      ["p", TEST_IDENTITIES.alice.pubkey, "", "buzz:audience:everyone"],
      ["p", TEST_IDENTITIES.bob.pubkey, "", "buzz:audience:everyone"],
      ["p", MIRA_PUBKEY, "", "buzz:audience:everyone"],
    ]);
});

test("@here snapshots only members whose active presence is online", async ({
  page,
}) => {
  const input = page.getByTestId("message-input");
  await input.fill("Online folks @he");

  const suggestion = page.getByTestId("mention-suggestion-audience-here");
  await expect(suggestion).toContainText(
    "Notify everyone online in this channel",
  );
  await suggestion.click();
  await page.getByTestId("send-message").click();

  await expect
    .poll(() => lastChannelMentionTags(page))
    .toEqual([
      ["buzz-audience-ref", "here"],
      ["p", TEST_IDENTITIES.alice.pubkey, "", "buzz:audience:here"],
      ["p", MIRA_PUBKEY, "", "buzz:audience:here"],
    ]);
});
