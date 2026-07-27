import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

// Typed ASCII emoticons (`:)`, `:P`, `<3`, ...) auto-replace with their
// unicode emoji the instant the pattern completes — see
// features/messages/lib/emoticonAutoReplace.ts.

async function openGeneral(page: Page) {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
}

test("typing :) auto-replaces with the emoji", async ({ page }) => {
  await installMockBridge(page);
  await openGeneral(page);

  const input = page.getByTestId("message-input");
  await input.click();
  await input.pressSequentially("hello :)");

  await expect(input).toHaveText("hello 🙂");
});

test("typing multiple emoticons in one message all convert", async ({
  page,
}) => {
  await installMockBridge(page);
  await openGeneral(page);

  const input = page.getByTestId("message-input");
  await input.click();
  await input.pressSequentially(":) :P <3");

  await expect(input).toHaveText("🙂 😛 ❤️");
});

test("does not convert an incomplete emoticon", async ({ page }) => {
  await installMockBridge(page);
  await openGeneral(page);

  const input = page.getByTestId("message-input");
  await input.click();
  await input.pressSequentially("hello :-");

  await expect(input).toHaveText("hello :-");
});

test("does not convert an emoticon glued to a preceding word", async ({
  page,
}) => {
  await installMockBridge(page);
  await openGeneral(page);

  const input = page.getByTestId("message-input");
  await input.click();
  await input.pressSequentially("hi:)");

  await expect(input).toHaveText("hi:)");
});

test("converts when the emoticon starts the message (nothing precedes it)", async ({
  page,
}) => {
  await installMockBridge(page);
  await openGeneral(page);

  const input = page.getByTestId("message-input");
  await input.click();
  await input.pressSequentially(":) hi");

  await expect(input).toHaveText("🙂 hi");
});

test("converts once a space separates it from the preceding word", async ({
  page,
}) => {
  await installMockBridge(page);
  await openGeneral(page);

  const input = page.getByTestId("message-input");
  await input.click();
  await input.pressSequentially("hi :)");

  await expect(input).toHaveText("hi 🙂");
});

test("typing :) followed by more text converts and keeps typing normally", async ({
  page,
}) => {
  await installMockBridge(page);
  await openGeneral(page);

  const input = page.getByTestId("message-input");
  await input.click();
  await input.pressSequentially("hi :) world");

  await expect(input).toHaveText("hi 🙂 world");
});

test("pasting text containing an emoticon does not convert it", async ({
  page,
}) => {
  await page.context().grantPermissions(["clipboard-read", "clipboard-write"], {
    origin: "http://127.0.0.1:4173",
  });

  await installMockBridge(page);
  await openGeneral(page);

  const input = page.getByTestId("message-input");
  await page.evaluate(
    (text) => navigator.clipboard.writeText(text),
    "pasted :) text",
  );
  await input.click();
  await page.keyboard.press("ControlOrMeta+V");

  await expect(input).toHaveText("pasted :) text");
});

test("does not convert an emoticon typed inside inline code", async ({
  page,
}) => {
  await installMockBridge(page);
  await openGeneral(page);

  const input = page.getByTestId("message-input");
  await input.click();
  // Toggle inline code formatting on (⌘E), type the emoticon, toggle off.
  await page.keyboard.press("ControlOrMeta+e");
  await input.pressSequentially(":)");
  await page.keyboard.press("ControlOrMeta+e");

  await expect(input).toHaveText(":)");
});
