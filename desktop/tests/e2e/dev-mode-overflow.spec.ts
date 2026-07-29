import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

// Developer-mode transcripts must keep chat content inside their pane: long
// unbroken words wrap, links truncate, and nothing pushes the shell wider
// than the window (the shell is a flex item — without min-w-0 its intrinsic
// min-content width lets nowrap content blow the layout out to the right).

const LONG_MESSAGE = `check https://example.com/${"averylongpathsegment-".repeat(12)}end and ${"Supercalifragilistic".repeat(14)} tail`;

async function expectNoHorizontalOverflow(
  page: import("@playwright/test").Page,
) {
  const overflow = await page.evaluate(() => {
    const rootWidth = document.documentElement.clientWidth;
    const shell = document.querySelector<HTMLElement>(
      '[data-testid="dev-mode-shell"]',
    );
    if (!shell) return { missing: true, offenders: [] as string[] };
    const offenders: string[] = [];
    const walk = (el: HTMLElement) => {
      // Ignore sub-pixel rounding; anything a pixel past the window is real.
      if (el.getBoundingClientRect().right > rootWidth + 1) {
        offenders.push(
          `${el.tagName.toLowerCase()}[${el.dataset.testid ?? ""}] right=${Math.round(el.getBoundingClientRect().right)}`,
        );
      }
      for (const child of el.children) walk(child as HTMLElement);
    };
    walk(shell);
    return { missing: false, offenders: offenders.slice(0, 10) };
  });
  expect(overflow.missing).toBe(false);
  expect(overflow.offenders).toEqual([]);
}

async function expectPaneContainsContent(
  page: import("@playwright/test").Page,
  testId: string,
) {
  const viewport = page.getByTestId(testId);
  const widths = await viewport.evaluate((el) => ({
    scrollWidth: el.scrollWidth,
    clientWidth: el.clientWidth,
  }));
  expect(widths.scrollWidth).toBeLessThanOrEqual(widths.clientWidth + 1);
}

test("dev-mode chat content stays inside its pane", async ({ page }) => {
  await page.setViewportSize({ width: 1000, height: 700 });
  await installMockBridge(page);
  await page.addInitScript(() => {
    localStorage.setItem("buzz.displayStyle", "developer");
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });

  const composer = page.getByTestId("dev-mode-composer");
  await composer.waitFor();

  // Open an existing channel: ArrowUp previews the newest channel, Enter opens.
  await composer.focus();
  await page.keyboard.press("ArrowUp");
  await page.keyboard.press("Enter");
  await page.getByTestId("dev-mode-transcript").waitFor();

  await composer.fill(LONG_MESSAGE);
  await page.keyboard.press("Enter");
  await expect(
    page.getByTestId("dev-mode-transcript").getByText("tail", { exact: false }),
  ).toBeVisible();

  await expectNoHorizontalOverflow(page);
  await expectPaneContainsContent(page, "dev-mode-transcript");

  // Side chat splits the screen; both panes must still contain their content.
  await page.keyboard.press("ArrowUp");
  await page.keyboard.press("Enter");
  await page.getByTestId("dev-mode-thread-panel").waitFor();

  await expectNoHorizontalOverflow(page);
  await expectPaneContainsContent(page, "dev-mode-transcript");
});
