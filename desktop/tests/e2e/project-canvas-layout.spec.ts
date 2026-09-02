import { expect, test, type Page } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

async function openStarterProject(page: Page) {
  const projectRow = page.getByTestId("sidebar-project-buzz");
  if ((await projectRow.count()) === 0) {
    await page.getByTestId("sidebar-projects-section-label").hover();
    await page.getByTestId("sidebar-projects-create").click();
    await page.getByTestId("project-browser-result-buzz").click();
  }
  await projectRow.click();
}

function storedLayoutKeys(page: Page) {
  return page.evaluate(() =>
    Object.keys(window.localStorage).filter((key) =>
      key.startsWith("buzz.projectCanvasLayout."),
    ),
  );
}

test("canvas widget layout survives a reload and is cleared by a reset", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");

  await openStarterProject(page);
  await expect(page.getByTestId("project-channel-home")).toBeVisible();

  const iframe = page.getByTestId("project-canvas-frame");
  const frame = page.frameLocator('[data-testid="project-canvas-frame"]');
  const root = frame.locator("#canvas-root");
  await expect(root).toHaveAttribute("data-canvas-ready", "true");
  await expect(root).toHaveAttribute("data-canvas-layouts", "{}");
  await expect(root).toHaveAttribute("data-canvas-widget-x", "0");
  await expect(root).toHaveAttribute("data-canvas-widget-width", "240");
  expect(await storedLayoutKeys(page)).toEqual([]);

  await frame.getByTestId("canvas-move-widget").click();
  await frame.getByTestId("canvas-move-widget").click();
  await expect(root).toHaveAttribute("data-canvas-widget-x", "48");
  await frame.getByTestId("canvas-resize-widget").click();
  await frame.getByTestId("canvas-resize-widget").click();
  await expect(root).toHaveAttribute("data-canvas-widget-width", "288");
  await expect
    .poll(() => storedLayoutKeys(page))
    .toEqual([expect.stringContaining("buzz.projectCanvasLayout.")]);

  // A fresh package activation mints a new load id and revision; layout is
  // keyed by project and dashboard, so it must survive both.
  const activeSource = await iframe.getAttribute("src");
  await page.getByTestId("project-canvas-reload").click();
  await expect(iframe).not.toHaveAttribute("src", activeSource ?? "");
  await expect(root).toHaveAttribute("data-canvas-ready", "true");
  await expect(root).toHaveAttribute("data-canvas-widget-x", "48");
  await expect(root).toHaveAttribute("data-canvas-widget-width", "288");

  await frame.getByTestId("canvas-reset-layout").click();
  await expect(root).toHaveAttribute("data-canvas-widget-x", "0");
  await expect(root).toHaveAttribute("data-canvas-widget-width", "240");
  await expect.poll(() => storedLayoutKeys(page)).toEqual([]);

  const resetSource = await iframe.getAttribute("src");
  await page.getByTestId("project-canvas-reload").click();
  await expect(iframe).not.toHaveAttribute("src", resetSource ?? "");
  await expect(root).toHaveAttribute("data-canvas-ready", "true");
  await expect(root).toHaveAttribute("data-canvas-layouts", "{}");
  await expect(root).toHaveAttribute("data-canvas-widget-x", "0");
  await expect(root).toHaveAttribute("data-canvas-widget-width", "240");
});
