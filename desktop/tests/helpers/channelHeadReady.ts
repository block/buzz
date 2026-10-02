import { expect, type Page } from "@playwright/test";

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
