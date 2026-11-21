import { expect, test } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

test("a failed send preserves an agent reply received before its rejection", async ({
  page,
}) => {
  await installMockBridge(page, {
    sendMessageErrors: ["Mock relay rejected the pending send"],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect
    .poll(() =>
      page.evaluate(() =>
        window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
          channelName: "general",
        }),
      ),
    )
    .toBe(true);

  // Hold only this send at the transport boundary; all incoming traffic uses
  // the normal mock relay and the production subscription handler.
  await page.evaluate(() => {
    const bridge = window.__TAURI_INTERNALS__;
    const invoke = bridge.invoke.bind(bridge);
    bridge.invoke = async (command, args) => {
      if (command === "plugin:websocket|send") {
        const payload = args as { message?: { data?: string } };
        const frame = JSON.parse(payload.message?.data ?? "null");
        if (
          frame?.[0] === "EVENT" &&
          frame[1]?.content === "pending test send"
        ) {
          await new Promise<void>((resolve) => {
            (
              window as Window & { rejectTestSend?: () => void }
            ).rejectTestSend = resolve;
          });
        }
      }
      return invoke(command, args);
    };
  });

  await page.getByTestId("message-input").fill("pending test send");
  await page.getByTestId("send-message").click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          typeof (window as Window & { rejectTestSend?: () => void })
            .rejectTestSend,
      ),
    )
    .toBe("function");
  await page.evaluate(() =>
    window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "general",
      content: "Completed agent result remains in the conversation",
    }),
  );
  const timeline = page.getByTestId("message-timeline");
  await expect(timeline).toContainText(
    "Completed agent result remains in the conversation",
  );
  await waitForAnimations(page);
  await timeline.screenshot({
    path: "test-results/failed-send-preserves-replies/before-rejection.png",
  });

  await page.evaluate(() =>
    (window as Window & { rejectTestSend?: () => void }).rejectTestSend?.(),
  );
  await expect(timeline).not.toContainText("pending test send");
  await expect(
    page.getByText("Mock relay rejected the pending send", { exact: false }),
  ).toBeVisible();
  await expect(timeline).toContainText(
    "Completed agent result remains in the conversation",
  );
  await waitForAnimations(page);
  await timeline.screenshot({
    path: "test-results/failed-send-preserves-replies/after-rejection.png",
  });

  await page.getByTestId("channel-random").click();
  await page.getByTestId("channel-general").click();
  await expect(timeline).toContainText(
    "Completed agent result remains in the conversation",
  );
});
