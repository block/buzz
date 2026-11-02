import { expect, type Locator, type Page } from "@playwright/test";

/** The seeded #general channel's id in the mock bridge. */
export const GENERAL_CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";

// These default mock fixtures have immediate EOSE and no injected reconnect.
// Steady-state pagination begins after live admission and its head refresh,
// which can replace paged tails. This does not test early startup scrolling.
export async function waitForMockChannelHeadReady(
  page: Page,
  channelName: string,
  channelId: string,
) {
  await page.waitForFunction(
    ({ channelId, channelName }) => {
      if (
        !window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
          channelName,
          kind: 39005,
        })
      ) {
        return false;
      }
      const commands = window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? [];
      const liveIndex = commands.findIndex(({ command, payload }) => {
        if (command !== "plugin:websocket|send") return false;
        const message = (
          payload as { message?: { type: string; data: string } }
        )?.message;
        if (message?.type !== "Text") return false;
        const [type, id, ...filters] = JSON.parse(message.data);
        return (
          type === "REQ" &&
          id.startsWith("live-") &&
          filters.some(
            (filter: { kinds?: number[]; "#h"?: string[] }) =>
              filter.kinds?.includes(39005) &&
              filter["#h"]?.includes(channelId),
          )
        );
      });
      const refreshed = commands
        .slice(liveIndex + 1)
        .some(({ command, payload }) => {
          const args = payload as {
            channelId?: string;
            cursor?: unknown;
          } | null;
          return (
            command === "get_channel_window" &&
            args?.channelId === channelId &&
            args.cursor === null
          );
        });
      const state = window.__BUZZ_E2E_QUERY_CLIENT__?.getQueryState([
        "channel-messages",
        channelId,
      ]);
      return (
        liveIndex >= 0 &&
        refreshed &&
        state?.status === "success" &&
        state.fetchStatus === "idle"
      );
    },
    { channelId, channelName },
  );
  const timeline = page.getByTestId("message-timeline");
  await expect(timeline.locator("[data-message-id]").first()).toBeVisible();
  await expect
    .poll(async () => {
      return timeline.evaluate(
        (element) =>
          element.scrollHeight - element.scrollTop - element.clientHeight,
      );
    })
    .toBeLessThanOrEqual(2);
}

// Opening #general refreshes the head after the live subscription lands, and
// the timeline re-pins while it does. Wait for that before touching a row.
export async function openGeneral(page: Page) {
  await page.getByTestId("channel-general").click();
  await waitForMockChannelHeadReady(page, "general", GENERAL_CHANNEL_ID);
}

// Row actions are hover-only, and a moving row slips out from under the
// pointer. The action bar is also shown on focus-within, so use the keyboard.
export async function pressRowAction(button: Locator) {
  await button.focus();
  await button.press("Enter");
}
