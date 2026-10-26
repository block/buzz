import { expect, test } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";

test("keeps a mic-less huddle active with screen sharing available", async ({
  page,
}) => {
  await page.addInitScript(() => {
    let requests = 0;
    Object.defineProperty(navigator.mediaDevices, "getUserMedia", {
      configurable: true,
      value: async () => {
        requests++;
        (
          window as Window & { microphoneRequests?: number }
        ).microphoneRequests = requests;
        throw new DOMException("Invalid constraint", "OverconstrainedError");
      },
    });
    Object.defineProperty(navigator.mediaDevices, "enumerateDevices", {
      configurable: true,
      value: async () => [],
    });
  });
  await installMockBridge(page, { openHuddleWindowDelayMs: 10_000 });
  await page.goto("/");
  await page.getByTestId("channel-alice-tyler").click();
  await page.getByTestId("channel-start-huddle-trigger").click();
  await expect(
    page.getByRole("button", { name: "Microphone unavailable", exact: true }),
  ).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(async () => {
        const state = (await window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.(
          "get_huddle_state",
        )) as { phase: string };
        return state.phase;
      }),
    )
    .toBe("active");
  await page.evaluate(async () => {
    await window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.("close_huddle_companion");
  });
  await expect(
    page.getByRole("button", { name: "Share screen", exact: true }),
  ).toBeEnabled();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as Window & { microphoneRequests?: number })
            .microphoneRequests,
      ),
    )
    .toBe(2);
  await expect(
    page.getByText("Invalid constraint", { exact: true }),
  ).toHaveCount(0);
  await page.getByRole("button", { name: "Leave huddle", exact: true }).click();
  await expect
    .poll(() =>
      page.evaluate(async () => {
        const state = (await window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.(
          "get_huddle_state",
        )) as { phase: string };
        return state?.phase ?? "idle";
      }),
    )
    .toBe("idle");
});

test("microphone permission refusal remains visible and rolls back the huddle", async ({
  page,
}) => {
  await page.addInitScript(() => {
    Object.defineProperty(navigator.mediaDevices, "getUserMedia", {
      configurable: true,
      value: async () => {
        throw new DOMException("Microphone access denied", "NotAllowedError");
      },
    });
  });
  await installMockBridge(page, { openHuddleWindowDelayMs: 10_000 });
  await page.goto("/");
  await page.getByTestId("channel-alice-tyler").click();
  await page.getByTestId("channel-start-huddle-trigger").click();
  await expect(
    page.getByText("Microphone access denied", { exact: true }),
  ).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(async () => {
        const state = (await window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.(
          "get_huddle_state",
        )) as { phase: string };
        return state?.phase ?? "idle";
      }),
    )
    .toBe("idle");
});
