import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

async function dispatchWheelPrevented(
  page: import("@playwright/test").Page,
  selector: string,
  deltas: { deltaX?: number; deltaY?: number },
) {
  return page.evaluate(
    ({ selector, deltaX, deltaY }) => {
      const element = document.querySelector(selector);
      if (!element) {
        throw new Error(`Missing element for selector: ${selector}`);
      }

      const event = new WheelEvent("wheel", {
        bubbles: true,
        cancelable: true,
        deltaX,
        deltaY,
      });
      element.dispatchEvent(event);
      return event.defaultPrevented;
    },
    { selector, deltaX: deltas.deltaX ?? 0, deltaY: deltas.deltaY ?? 0 },
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
    dispatchWheelPrevented(page, '[data-testid="app-top-chrome"]', {
      deltaY: -120,
    }),
  ).resolves.toBe(true);
  await expect(
    dispatchWheelPrevented(page, '[data-testid="sidebar-pinned-header"]', {
      deltaY: -120,
    }),
  ).resolves.toBe(true);
  await expect(
    dispatchWheelPrevented(page, '[data-testid="app-sidebar-scroll-anchor"]', {
      deltaY: -120,
    }),
  ).resolves.toBe(true);
  await expect(
    dispatchWheelPrevented(page, '[data-testid="chat-title"]', {
      deltaY: -120,
    }),
  ).resolves.toBe(true);

  await expect(
    dispatchWheelPrevented(page, '[data-testid="message-timeline"]', {
      deltaY: -120,
    }),
  ).resolves.toBe(false);

  // Buzz Term consumes wheel gestures as custom scrollback rather than through
  // a native scroll container. The viewport lock must leave that vertical
  // gesture alone so the substrate's own handler can receive it.
  await page.evaluate(() => {
    const terminal = document.createElement("section");
    terminal.dataset.terminalOwner = "terminal";
    terminal.dataset.testid = "terminal-wheel-target";
    document.body.append(terminal);
  });
  await expect(
    dispatchWheelPrevented(page, '[data-testid="terminal-wheel-target"]', {
      deltaY: -120,
    }),
  ).resolves.toBe(false);
});

test("locks horizontal viewport pan everywhere", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("message-timeline")).toBeVisible();

  for (const deltaX of [-120, 120]) {
    await expect(
      dispatchWheelPrevented(page, '[data-testid="app-top-chrome"]', {
        deltaX,
      }),
    ).resolves.toBe(true);
    await expect(
      dispatchWheelPrevented(page, '[data-testid="sidebar-pinned-header"]', {
        deltaX,
      }),
    ).resolves.toBe(true);
    await expect(
      dispatchWheelPrevented(page, '[data-testid="chat-title"]', { deltaX }),
    ).resolves.toBe(true);

    // Unlike vertical, horizontal pans over the conversation pane are locked
    // too — there is no horizontal elastic affordance.
    await expect(
      dispatchWheelPrevented(page, '[data-testid="message-timeline"]', {
        deltaX,
      }),
    ).resolves.toBe(true);
  }

  await page.evaluate(() => {
    const terminal = document.createElement("section");
    terminal.dataset.terminalOwner = "terminal";
    terminal.dataset.testid = "terminal-wheel-target";
    document.body.append(terminal);
  });
  for (const [deltaX, deltaY, prevented] of [
    [120, 0, true],
    [0, 120, false],
    [120, 120, false],
    [120, 20, true],
  ] as const) {
    await expect(
      dispatchWheelPrevented(page, '[data-testid="terminal-wheel-target"]', {
        deltaX,
        deltaY,
      }),
    ).resolves.toBe(prevented);
  }

  // A concealed substrate is not an active custom wheel consumer. Keep dead
  // space locked even if a future layout places Buzz content inside it.
  await page.evaluate(() => {
    const terminal = document.querySelector<HTMLElement>(
      '[data-testid="terminal-wheel-target"]',
    );
    const buzzContent = document.createElement("div");
    buzzContent.dataset.testid = "concealed-terminal-dead-space";
    terminal?.append(buzzContent);
    if (terminal) terminal.dataset.terminalOwner = "buzz";
  });
  await expect(
    dispatchWheelPrevented(
      page,
      '[data-testid="concealed-terminal-dead-space"]',
      { deltaY: 120 },
    ),
  ).resolves.toBe(true);

  // A predominantly vertical gesture with slight horizontal drift still
  // reaches the conversation scroller.
  await expect(
    dispatchWheelPrevented(page, '[data-testid="message-timeline"]', {
      deltaX: -10,
      deltaY: -120,
    }),
  ).resolves.toBe(false);
});
