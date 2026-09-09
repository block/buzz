import { expect, test } from "@playwright/test";

test("table cells align to the reading edge and scroll surfaces use thin bars", async ({
  page,
}) => {
  await page.goto("/design/components");
  const cells = page.locator("#table th, #table td");
  for (const cell of await cells.all())
    await expect(cell).toHaveCSS("text-align", "start");
  for (const selector of ["html", "textarea", "#table .bui-table-scroll"]) {
    await expect(page.locator(selector).first()).toHaveCSS(
      "scrollbar-width",
      "thin",
    );
  }
  const viewport = page.locator("#scroll-area .bui-scroll-viewport");
  // Custom viewports must continue hiding the native bar, avoiding double scrollbars.
  await expect(viewport).toHaveCSS("scrollbar-width", "none");
  await expect(page.locator("#scroll-area .bui-scrollbar")).toHaveCSS(
    "width",
    "6px",
  );
  await viewport.press("End");
  await expect
    .poll(() => viewport.evaluate((node) => node.scrollTop))
    .toBeGreaterThan(0);
  await page.goto("/design/components/input");
  await page.getByText("View example module", { exact: true }).click();
  await expect(page.locator(".catalog-source pre")).toHaveCSS(
    "scrollbar-width",
    "thin",
  );
});

test("resize grip is one rem with a usable pointer and keyboard target", async ({
  page,
}) => {
  await page.goto("/design/components");
  const handle = page.getByRole("separator", { name: "Resize project list" });
  await handle.scrollIntoViewIfNeeded();
  const grip = () =>
    handle.evaluate((node) => getComputedStyle(node, "::after").height);
  expect(await grip()).toBe("16px");
  const bounds = await handle.boundingBox();
  if (!bounds) throw new Error("Missing resize target");
  expect(bounds.width).toBeGreaterThanOrEqual(24);
  expect(bounds.height).toBeGreaterThan(16);
  const before = await handle.getAttribute("aria-valuenow");
  await page.mouse.move(
    bounds.x + bounds.width / 2,
    bounds.y + bounds.height / 2,
  );
  await page.mouse.down();
  await page.mouse.move(
    bounds.x + bounds.width / 2 + 40,
    bounds.y + bounds.height / 2,
    { steps: 5 },
  );
  await page.mouse.up();
  await expect(handle).not.toHaveAttribute("aria-valuenow", before ?? "");
  const after = await handle.getAttribute("aria-valuenow");
  await handle.press("ArrowLeft");
  await expect(handle).not.toHaveAttribute("aria-valuenow", after ?? "");
  await page.addStyleTag({ content: "html { font-size: 20px; }" });
  expect(await grip()).toBe("20px");
});

test("Glass navigation shares selection motion and keyboard semantics", async ({
  page,
}) => {
  await page.goto("/design/glass");
  const tabs = page.getByRole("tablist", { name: "Glass navigation preview" });
  const indicator = tabs.locator(".bui-tab-indicator");
  await expect(page.getByRole("tabpanel")).toHaveAttribute("tabindex", "-1");
  await expect(
    tabs.getByRole("tab", { name: "Messages", exact: true }),
  ).toHaveAttribute("aria-selected", "true");
  await page.addStyleTag({
    content: ".bui-tab-indicator { transition-duration: 2s; }",
  });
  await tabs.getByRole("tab", { name: "Projects", exact: true }).click();
  await expect
    .poll(() =>
      indicator.evaluate((node) =>
        node
          .getAnimations()
          .map((a) => (a as CSSTransition).transitionProperty),
      ),
    )
    .toContain("clip-path");
  await tabs.getByRole("tab", { name: "Me", exact: true }).click();
  await expect(
    tabs.getByRole("tab", { name: "Me", exact: true }),
  ).toHaveAttribute("aria-selected", "true");
  await tabs.getByRole("tab", { name: "Me", exact: true }).press("ArrowRight");
  await expect(
    tabs.getByRole("tab", { name: "Messages", exact: true }),
  ).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(
    tabs.getByRole("tab", { name: "Messages", exact: true }),
  ).toHaveAttribute("aria-selected", "true");
  expect(await indicator.evaluate((node) => node.getAnimations().length)).toBe(
    0,
  );
  await page.emulateMedia({ reducedMotion: "reduce" });
  await tabs.getByRole("tab", { name: "Projects", exact: true }).click();
  await expect(
    tabs.getByRole("tab", { name: "Projects", exact: true }),
  ).toHaveAttribute("aria-selected", "true");
  expect(await indicator.evaluate((node) => node.getAnimations().length)).toBe(
    0,
  );
});
