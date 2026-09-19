import { expect, test } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";

test("unknown channel references render inert chips beside known channels", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await page.waitForFunction(() =>
    window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
      channelName: "general",
    }),
  );
  const id = await page.evaluate(() => {
    const message = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "general",
      content:
        "Channel reference regression: #general and #getting-started. Keep `#code-room` and https://example.com/a#fragment unchanged.",
    });
    if (!message) throw new Error("Mock message was not emitted");
    return message.id;
  });
  const message = page
    .getByTestId("message-timeline")
    .locator(`[data-message-id="${id}"]`);
  const chips = message.locator("[data-channel-link]");
  await expect(chips).toHaveCount(2);
  await expect(chips.nth(0)).toHaveText("general");
  await expect(chips.nth(1)).toHaveText("getting-started");
  await expect(chips.nth(1)).not.toHaveAttribute("href");
  await expect(message.locator("code")).toHaveText("#code-room");
  await expect(
    message.getByRole("link", {
      name: "https://example.com/a#fragment",
      exact: true,
    }),
  ).toHaveAttribute("href", "https://example.com/a#fragment");
});
