import { expect, test } from "@playwright/test";

import { TEST_IDENTITIES, installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

const SHOTS = "test-results/channel-mentions";

test.use({ viewport: { width: 1280, height: 720 } });

async function waitForMockLiveSubscription(
  page: import("@playwright/test").Page,
  channelName: string,
) {
  await expect
    .poll(() =>
      page.evaluate(
        ({ ch }) =>
          (
            window as Window & {
              __BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?: (input: {
                channelName: string;
              }) => boolean;
            }
          ).__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({ channelName: ch }) ??
          false,
        { ch: channelName },
      ),
    )
    .toBe(true);
}

function emitNotifyMessage(
  page: import("@playwright/test").Page,
  channelName: string,
  content: string,
  notifyMode: "channel" | "here" | null,
) {
  return page.evaluate(
    ({ ch, msg, pubkey, mode }) => {
      (
        window as Window & {
          __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
            channelName: string;
            content: string;
            pubkey: string;
            extraTags?: string[][];
          }) => unknown;
        }
      ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: ch,
        content: msg,
        pubkey,
        extraTags: mode ? [["notify", mode]] : undefined,
      });
    },
    {
      ch: channelName,
      msg: content,
      pubkey: TEST_IDENTITIES.alice.pubkey,
      mode: notifyMode,
    },
  );
}

test("@channel and @here autocomplete rows offer the reserved tokens", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId("channel-general").click();

  const composer = page.getByTestId("message-composer");
  const input = composer.getByTestId("message-input");
  await input.fill("Heads up @ch");

  const autocomplete = composer.getByTestId("mention-autocomplete");
  const channelRow = autocomplete.getByTestId("mention-suggestion-channel");
  await expect(channelRow).toContainText("@channel");
  await expect(channelRow).toContainText("Notify everyone in this channel");

  await waitForAnimations(page);
  await autocomplete.screenshot({
    path: `${SHOTS}/01-channel-suggestion.png`,
  });

  await input.fill("Heads up @he");
  const hereRow = autocomplete.getByTestId("mention-suggestion-here");
  await expect(hereRow).toContainText("@here");
  await expect(hereRow).toContainText("Notify members who are online");

  await waitForAnimations(page);
  await autocomplete.screenshot({ path: `${SHOTS}/02-here-suggestion.png` });
});

test("sending @channel asks for confirmation first", async ({ page }) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId("channel-general").click();

  const composer = page.getByTestId("message-composer");
  const input = composer.getByTestId("message-input");
  await input.fill("@channel standup moved to 10am");
  // Dismiss the autocomplete so Enter sends instead of accepting a suggestion.
  await input.press("Escape");
  await input.press("Enter");

  const confirm = page.getByTestId("channel-notify-confirm");
  await expect(confirm).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/03-confirm-dialog.png` });

  await confirm.click();
  await expect(confirm).toBeHidden();
});

test("@channel chips only on messages that carry the notify tag", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await waitForMockLiveSubscription(page, "general");

  await emitNotifyMessage(
    page,
    "general",
    "@channel deploy at noon",
    "channel",
  );
  await emitNotifyMessage(page, "general", "@channel is just text here", null);

  const tagged = page
    .locator(".message-markdown", { hasText: "deploy at noon" })
    .first();
  await expect(tagged.locator("[data-mention]")).toHaveText("@channel");

  const untagged = page
    .locator(".message-markdown", { hasText: "is just text here" })
    .first();
  await expect(untagged.locator("[data-mention]")).toHaveCount(0);

  await waitForAnimations(page);
  await tagged.screenshot({ path: `${SHOTS}/04-channel-pill.png` });
});
