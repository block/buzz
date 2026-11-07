import { expect, test } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

const AGENT =
  "554cef57437abac34522ac2c9f0490d685b72c80478cf9f7ed6f9570ee8624ea";

test("Forum thread shows typing, excludes other posts, and expires stopped heartbeats", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId("channel-watercooler").click();
  await page
    .getByText("Release checklist: async feedback thread.", { exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "Back to posts" }),
  ).toBeVisible();
  const emit = (threadHeadId: string) =>
    page.evaluate(
      ({ threadHeadId, pubkey }) => {
        window.__BUZZ_E2E_EMIT_MOCK_TYPING__?.({
          channelName: "watercooler",
          pubkey,
          threadHeadId,
        });
      },
      { threadHeadId, pubkey: AGENT },
    );
  await emit("mock-forum-offsite-thread");
  await expect(page.getByTestId("message-typing-indicator")).toHaveCount(0);
  // Retry emission until the asynchronous subscription is established.
  await expect
    .poll(async () => {
      await emit("mock-forum-release-thread");
      return page.getByTestId("message-typing-indicator").count();
    })
    .toBe(1);
  await expect(
    page.getByTestId("message-typing-indicator-label"),
  ).toContainText("is typing");
  await waitForAnimations(page);
  await page.screenshot({ path: "test-results/forum-typing-working.png" });
  await expect(page.getByTestId("message-typing-indicator")).toHaveCount(0, {
    timeout: 11_000,
  });
});
