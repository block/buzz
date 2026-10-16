import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

/**
 * WebKit scrolls on the compositor only where no non-passive wheel listener
 * can cancel the gesture. A single `{ passive: false }` wheel listener on
 * window, document, or any ancestor of the timeline puts every wheel tick
 * over the conversation behind the main thread, so the scroll waits for
 * whatever React is doing. The viewport rubber-band lock is therefore CSS on
 * the document root, and the timeline's scroll path must stay free of
 * cancelable wheel listeners.
 */

declare global {
  interface Window {
    __BUZZ_E2E_NON_PASSIVE_WHEEL_TARGETS__?: EventTarget[];
  }
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    const targets: EventTarget[] = [];
    window.__BUZZ_E2E_NON_PASSIVE_WHEEL_TARGETS__ = targets;
    const original = EventTarget.prototype.addEventListener;
    EventTarget.prototype.addEventListener = function patched(
      this: EventTarget,
      type: string,
      listener: EventListenerOrEventListenerObject | null,
      options?: boolean | AddEventListenerOptions,
    ) {
      const passive =
        typeof options === "object" && options !== null && options.passive;
      if (type === "wheel" && !passive) targets.push(this);
      return original.call(this, type, listener, options);
    };
  });
  await installMockBridge(page);
});

test("the timeline's scroll path has no non-passive wheel listener", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("message-timeline")).toBeVisible();

  const offenders = await page.evaluate(() => {
    const timeline = document.querySelector('[data-testid="message-timeline"]');
    if (!timeline) throw new Error("timeline missing");
    const describe = (target: EventTarget) => {
      if (target === window) return "window";
      if (target === document) return "document";
      if (!(target instanceof Element)) return String(target);
      const testId = target.getAttribute("data-testid");
      return `${target.tagName.toLowerCase()}${testId ? `[data-testid=${testId}]` : ""}`;
    };
    return (window.__BUZZ_E2E_NON_PASSIVE_WHEEL_TARGETS__ ?? [])
      .filter(
        (target) =>
          target === window ||
          target === document ||
          (target instanceof Node && target.contains(timeline)),
      )
      .map(describe);
  });
  expect(offenders).toEqual([]);
});

test("the document root carries the viewport rubber-band lock", async ({
  page,
}) => {
  await page.goto("/");
  await expect(page.getByTestId("channel-general")).toBeVisible();

  const rootStyles = await page.evaluate(() =>
    [document.documentElement, document.body].map((element) => {
      const style = getComputedStyle(element);
      return {
        overflow: style.overflow,
        overscrollBehavior: style.overscrollBehavior,
      };
    }),
  );
  for (const style of rootStyles) {
    expect(style.overflow).toBe("hidden");
    expect(style.overscrollBehavior).toBe("none");
  }
});
