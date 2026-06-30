import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

async function dispatchWheelPrevented(
  page: import("@playwright/test").Page,
  selector: string,
  deltaY: number,
) {
  return page.evaluate(
    ({ selector, deltaY }) => {
      const element = document.querySelector(selector);
      if (!element) {
        throw new Error(`Missing element for selector: ${selector}`);
      }

      const event = new WheelEvent("wheel", {
        bubbles: true,
        cancelable: true,
        deltaY,
      });
      element.dispatchEvent(event);
      return event.defaultPrevented;
    },
    { selector, deltaY },
  );
}

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

test("locks viewport rubber-band outside conversation scrollers", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("message-timeline")).toBeVisible();

  await expect(
    dispatchWheelPrevented(page, '[data-testid="app-top-chrome"]', -120),
  ).resolves.toBe(true);
  await expect(
    dispatchWheelPrevented(page, '[data-testid="sidebar-pinned-header"]', -120),
  ).resolves.toBe(true);
  await expect(
    dispatchWheelPrevented(
      page,
      '[data-testid="app-sidebar-scroll-anchor"]',
      -120,
    ),
  ).resolves.toBe(true);
  await expect(
    dispatchWheelPrevented(page, '[data-testid="chat-title"]', -120),
  ).resolves.toBe(true);

  await expect(
    dispatchWheelPrevented(page, '[data-testid="message-timeline"]', -120),
  ).resolves.toBe(false);
});
