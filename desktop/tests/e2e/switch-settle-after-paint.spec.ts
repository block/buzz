import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

/**
 * The switch trace must settle AFTER the deferred timeline has committed and
 * painted. During the skeleton→loaded transition the message-list branches
 * (which used to own the `data-render-pending` marker) are not mounted, so a
 * tracer polling only that marker would read "not pending" and record a
 * settle while the heavy deferred list was still uncommitted — underreporting
 * exactly the switches the tracer exists to measure. The marker now lives on
 * the timeline's always-mounted wrapper; this spec pins the contract on a
 * real empty→loaded cold switch into a deep channel.
 */

const SWITCH_MEASURE = "buzz:channel-switch:click-to-settled";

test("cold-switch settle measure lands only after rows are painted", async ({
  page,
}) => {
  await installMockBridge(page, { deepHistoryMessageCount: 600 });
  await page.goto("/");
  await expect(page.getByTestId("app-sidebar")).toBeVisible();

  // Cold first entry: skeleton → deferred list commit → settled paint.
  await page.getByTestId("channel-deep-history").click();

  // Poll for the settle measure inside the page and — in the same synchronous
  // evaluation turn — snapshot what the DOM shows at that moment. Reading the
  // DOM from the test process after the fact would race further renders.
  const atSettle = await page.evaluate(async (measureName) => {
    const deadline = Date.now() + 15_000;
    while (Date.now() < deadline) {
      if (performance.getEntriesByName(measureName).length > 0) {
        return {
          renderPending:
            document.querySelector('[data-render-pending="true"]') !== null,
          rowCount: document.querySelectorAll(
            '[data-message-id^="mock-deep-history-"]',
          ).length,
          settled: true,
        };
      }
      await new Promise((resolve) => setTimeout(resolve, 16));
    }
    return { renderPending: true, rowCount: 0, settled: false };
  }, SWITCH_MEASURE);

  expect(atSettle.settled, "switch trace must settle").toBe(true);
  expect(
    atSettle.rowCount,
    "settle must not be recorded before the deferred list painted",
  ).toBeGreaterThan(0);
  expect(
    atSettle.renderPending,
    "settle must not be recorded while a deferred commit is still pending",
  ).toBe(false);
});
