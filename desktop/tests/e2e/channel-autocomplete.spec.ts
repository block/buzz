import { expect, test, type Page } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

async function openComposer(
  page: Page,
  surface: "channel" | "thread" | "forum",
) {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  if (surface === "thread") {
    await page.locator('[data-message-id="mock-general-alice"]').hover();
    await page.getByTestId("reply-message-mock-general-alice").click();
    return page
      .getByTestId("message-thread-panel")
      .getByTestId("message-input");
  }
  if (surface === "forum") {
    await page.getByTestId("channel-watercooler").click();
    await page.getByRole("button", { name: "Start a new post..." }).click();
  }
  return page.getByTestId("message-input");
}

function channelSuggestion(page: Page) {
  return page.getByRole("button", { name: "#general stream", exact: true });
}

for (const surface of ["channel", "thread", "forum"] as const) {
  test(`Enter selects a channel in the ${surface} composer without splitting or submitting`, async ({
    page,
  }) => {
    const input = await openComposer(page, surface);
    await input.pressSequentially("#gener");
    await expect(channelSuggestion(page)).toBeVisible();
    await input.press("Enter");

    // Assert literal text (including the separator), document shape and focus:
    // hook-only tests cannot catch ProseMirror splitting before React handles Enter.
    await expect.poll(() => input.textContent()).toBe("#general ");
    await expect(input.locator("p")).toHaveCount(1);
    await expect(input.locator("br")).toHaveCount(0);
    await expect(input).toBeFocused();
    await expect(channelSuggestion(page)).toBeHidden();

    // Autocomplete must relinquish the next Enter to ordinary submission.
    await page.keyboard.type("selected channel");
    await input.press("Enter");
    await expect
      .poll(() =>
        page.evaluate(() => {
          const content = "#general selected channel";
          const signed = (window.__BUZZ_E2E_SIGNED_EVENTS__ ?? []).filter(
            (event) => event.content === content,
          );
          if (signed.length) return signed.length;
          return (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).filter(
            (call) =>
              call.command === "send_channel_message" &&
              (call.payload as { content?: string }).content === content,
          ).length;
        }),
      )
      .toBe(1);
  });
}

for (const key of ["Tab", "Escape", "Shift+Enter"] as const) {
  test(`channel autocomplete preserves ${key} behavior`, async ({ page }) => {
    const input = await openComposer(page, "channel");
    await input.fill("#gener");
    await expect(channelSuggestion(page)).toBeVisible();
    await input.press(key);
    await expect
      .poll(() => input.textContent())
      .toBe(key === "Tab" ? "#general " : "#gener");
    await expect(channelSuggestion(page)).toBeHidden();
    await expect(input.locator("p")).toHaveCount(1);
    if (key === "Shift+Enter") {
      await page.keyboard.type("next line");
      await expect(input).toHaveText("#genernext line");
      await expect(input.locator("br")).toHaveCount(1);
    } else {
      await expect(input.locator("br")).toHaveCount(0);
    }
  });
}

for (const [prefix, expected] of [
  ["@ali", "@alice "],
  [":joy", "😂 "],
] as const) {
  test(`Enter still selects ${prefix} autocomplete without splitting`, async ({
    page,
  }) => {
    const input = await openComposer(page, "channel");
    const dropdown = page.getByTestId(
      prefix === "@ali" ? "mention-autocomplete" : "emoji-autocomplete",
    );
    await input.fill(prefix);
    await expect(dropdown).toBeVisible();
    await input.press("Enter");
    await expect(dropdown).toBeHidden();
    await expect.poll(() => input.textContent()).toBe(expected);
    await expect(input.locator("p")).toHaveCount(1);
    await expect(input.locator("br")).toHaveCount(0);
    await expect(input).toBeFocused();
  });
}
