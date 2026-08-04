import { expect, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

// Typing `@` in a developer-mode composer autocompletes users: channel
// members rank first, relay-wide search fills in the rest. Accepting a
// suggestion keeps the literal `@Display Name` in the text, and sending
// emits a ["p", pubkey] tag for it. Mentioning a non-member best-effort
// adds them to the channel before the send.

async function openDevModeChannel(
  page: import("@playwright/test").Page,
  channelName: string,
) {
  await installMockBridge(page);
  await page.addInitScript(() => {
    localStorage.setItem("buzz.displayStyle", "developer");
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });

  const composer = page.getByTestId("dev-mode-composer");
  await composer.waitFor();
  await composer.focus();

  // ArrowUp steps through channel previews newest-first; walk until the
  // requested channel is previewed, then Enter opens it.
  const topBar = page.getByTestId("dev-mode-topbar-channel");
  for (let step = 0; step < 20; step += 1) {
    await page.keyboard.press("ArrowUp");
    const previewed = (await topBar.innerText()).replace(/^#\s*/, "").trim();
    if (previewed === channelName) break;
  }
  await expect(topBar).toContainText(channelName);
  await page.keyboard.press("Enter");
  await page.getByTestId("dev-mode-transcript").waitFor();
  return composer;
}

test("composer @-mention autocompletes a relay user into a p-tagged send", async ({
  page,
}) => {
  // outsider exists on the relay but is not a member of #general.
  const composer = await openDevModeChannel(page, "general");

  await composer.pressSequentially("hey @out");
  const suggestions = page.getByTestId("dev-mode-mention-suggestions");
  await expect(suggestions).toBeVisible();
  await expect(suggestions).toContainText("outsider");
  await expect(suggestions).toContainText("adds to channel");

  await page.keyboard.press("Tab");
  await expect(suggestions).not.toBeVisible();
  await expect(composer).toHaveValue(/@outsider /);

  const marker = `please review ${Date.now()}`;
  await composer.pressSequentially(marker);
  await page.keyboard.press("Enter");

  const transcript = page.getByTestId("dev-mode-transcript");
  await expect(transcript).toContainText("@outsider");
  await expect(transcript).toContainText(marker);

  // The signed outgoing event carries the mention p tag.
  await expect
    .poll(() =>
      page.evaluate((needle) => {
        const events = (
          window as Window & {
            __BUZZ_E2E_SIGNED_EVENTS__?: Array<{
              content: string;
              tags: string[][];
            }>;
          }
        ).__BUZZ_E2E_SIGNED_EVENTS__;
        return (
          events?.find((event) => event.content.includes(needle))?.tags ?? []
        );
      }, marker),
    )
    .toContainEqual(["p", TEST_IDENTITIES.outsider.pubkey]);

  // Mentioning a non-member pulls them into the channel best-effort.
  const commandLog = await page.evaluate(() => {
    return (
      (
        window as Window & {
          __BUZZ_E2E_COMMAND_LOG__?: Array<{
            command: string;
            payload: unknown;
          }>;
        }
      ).__BUZZ_E2E_COMMAND_LOG__ ?? []
    );
  });
  expect(commandLog).toEqual(
    expect.arrayContaining([
      expect.objectContaining({
        command: "add_channel_members",
        payload: expect.objectContaining({
          pubkeys: expect.arrayContaining([TEST_IDENTITIES.outsider.pubkey]),
        }),
      }),
    ]),
  );
});

test("mid-word @ (email addresses) does not open mention autocomplete", async ({
  page,
}) => {
  const composer = await openDevModeChannel(page, "general");

  await composer.pressSequentially("reach me at joe@out");
  await expect(
    page.getByTestId("dev-mode-mention-suggestions"),
  ).not.toBeVisible();
});

test("side-chat composer @-mention suggests channel members", async ({
  page,
}) => {
  await openDevModeChannel(page, "general");

  // ArrowUp selects the newest card; Enter opens its side chat.
  await page.keyboard.press("ArrowUp");
  await page.keyboard.press("Enter");
  await page.getByTestId("dev-mode-thread-panel").waitFor();

  const threadComposer = page.getByTestId("dev-mode-thread-composer");
  await threadComposer.focus();
  await threadComposer.pressSequentially("@ali");

  const suggestions = page.getByTestId("dev-mode-mention-suggestions");
  await expect(suggestions).toBeVisible();
  await expect(suggestions).toContainText("alice");

  await page.keyboard.press("Tab");
  await expect(threadComposer).toHaveValue(/@alice /);
});
