import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

// Custom-emoji end-to-end guard.
//
// The composer renders a known `:shortcode:` as a real inline atom node
// (`img[data-custom-emoji]`) that selects/copies/deletes as one unit, while
// still serializing to `:shortcode:` on send. The message timeline renders the
// same shortcode as `img[data-custom-emoji]` via remarkCustomEmoji.
//
// The `:sprout:` shortcode lives in a member-authored kind:30030 set
// (d=`sprout:custom-emoji`) served by the mock bridge from two distinct
// pubkeys. `listCustomEmoji` reads every member's set over the relay WS and
// unions them (deduped by shortcode+url) into the workspace palette — which is
// live even in mock-bridge mode (the mock only intercepts Tauri commands), so
// this spec uses the simpler mock-bridge setup like messaging.spec.ts.
const SHORTCODE = "sprout";

async function openGeneral(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
}

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

test("typing a known :shortcode: renders an inline emoji node in the composer", async ({
  page,
}) => {
  await openGeneral(page);

  const input = page.getByTestId("message-input");
  await input.click();
  // pressSequentially (not fill) so the node input rule fires on the final ":".
  await input.pressSequentially(`:${SHORTCODE}:`);

  const node = input.locator("img[data-custom-emoji]");
  await expect(node).toHaveCount(1);
  await expect(node).toHaveAttribute("alt", `:${SHORTCODE}:`);
  await expect(node).toHaveAttribute("data-shortcode", SHORTCODE);
  // The raw text must NOT linger alongside the node.
  await expect(input).not.toContainText(`:${SHORTCODE}:`);
});

test("custom emoji deletes as a single unit (like a built-in emoji)", async ({
  page,
}) => {
  await openGeneral(page);

  const input = page.getByTestId("message-input");
  await input.click();
  await input.pressSequentially(`hi :${SHORTCODE}:`);

  const node = input.locator("img[data-custom-emoji]");
  await expect(node).toHaveCount(1);

  // One backspace at the end removes the whole atom node, not a character of
  // hidden text.
  await input.press("Backspace");
  await expect(node).toHaveCount(0);
  await expect(input).toContainText("hi");
});

test("custom emoji round-trips through select-all + send to the timeline", async ({
  page,
}) => {
  await openGeneral(page);

  const input = page.getByTestId("message-input");
  await input.click();
  await input.pressSequentially(`:${SHORTCODE}:`);
  await expect(input.locator("img[data-custom-emoji]")).toHaveCount(1);

  // Select-all then a single delete clears the node as one unit, proving it is
  // part of the selectable document (the bug was the caret skipping it).
  await input.press("ControlOrMeta+a");
  await input.press("Backspace");
  await expect(input.locator("img[data-custom-emoji]")).toHaveCount(0);

  // Re-enter and send: it must serialize to `:shortcode:` and re-render as an
  // <img> in the timeline (remarkCustomEmoji), not as raw text.
  await input.pressSequentially(`:${SHORTCODE}:`);
  await expect(input.locator("img[data-custom-emoji]")).toHaveCount(1);
  await page.getByTestId("send-message").click();

  const sentEmoji = page
    .getByTestId("message-timeline")
    .locator(`img[data-custom-emoji][alt=":${SHORTCODE}:"]`);
  await expect(sentEmoji.last()).toBeVisible();
  // The composer clears after send.
  await expect(input.locator("img[data-custom-emoji]")).toHaveCount(0);
});
