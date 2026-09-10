import { expect, test } from "@playwright/test";

/**
 * The workspace's layout seams and its agreement with the colour-scheme fact.
 *
 * Each assertion here corresponds to a defect that had already shipped, and
 * each is falsifiable: removing the production rule turns the test red. That
 * matters most for the two that no screenshot would catch — a resize handle too
 * narrow to grab, and a vendor theme that disagrees with the app's mode.
 */

test("the panels are separated by a gutter with a grabbable seam inside it", async ({
  page,
}) => {
  await page.goto("/?mock=1");
  await page.getByRole("button", { name: "design", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "design", exact: true }),
  ).toBeVisible();

  const geometry = await page.evaluate(() => {
    const views = [
      ...document.querySelectorAll(
        ".buzz-dockview .dv-horizontal > .dv-view-container > .dv-view",
      ),
    ];
    const sash = document.querySelector(".buzz-dockview .dv-sash");
    if (views.length < 2 || !sash) return null;
    const [first, second] = views.map((view) =>
      view.getBoundingClientRect(),
    ) as [DOMRect, DOMRect];
    const seam = sash.getBoundingClientRect();
    return {
      gutter: second.left - first.right,
      gutterCentre: (first.right + second.left) / 2,
      seamWidth: seam.width,
      seamCentre: (seam.left + seam.right) / 2,
      cursor: getComputedStyle(sash).cursor,
    };
  });
  if (!geometry) throw new Error("The workspace did not render two panels.");

  // The air between panels is the dockview theme's `gap`, fed from
  // `--space-panel-gap`. Each group used to carry inset padding instead, which
  // looked the same and left no seam in the gutter to grab.
  expect(geometry.gutter).toBeGreaterThan(4);

  // The grab area is deliberately wider than the gutter it sits in: an 8px
  // invisible seam is not a hit target. Dockview's own width is 4px, so this
  // fails if our rule stops applying — which is exactly what happened when the
  // vendor stylesheet was imported unlayered and beat every rule we authored.
  expect(geometry.seamWidth).toBeGreaterThan(8);

  // Widening it must be paid for on both sides. Measured against the gutter
  // rather than dockview's own width, the seam lands 2px off centre.
  expect(Math.abs(geometry.seamCentre - geometry.gutterCentre)).toBeLessThan(1);

  // Dockview sets the axis cursor per orientation. A hardcoded `col-resize`
  // would be wrong on every horizontal seam.
  expect(geometry.cursor).toBe("ew-resize");
});

test("the workspace layout follows the one colour-scheme answer", async ({
  page,
}) => {
  await page.goto("/?mock=1");
  await page.evaluate(() => localStorage.removeItem("buzz-next-color-scheme"));
  await page.reload();

  const dockTheme = () =>
    page
      .locator("[class*=dockview-theme]")
      .first()
      .getAttribute("class")
      .then((value) => value ?? "");

  await page.getByRole("button", { name: "Use dark mode" }).click();
  await expect(page.locator("html")).toHaveClass(/dark/);

  // The real assertion. `useColorScheme` used to hold per-caller `useState`, so
  // the shell's toggle updated only the shell's copy. The `.dark` class still
  // flipped — the shell writes it — so everything reading the mode through CSS
  // looked right and this defect was invisible. Dockview takes its theme as a
  // JavaScript object, so it kept being handed the light theme on a dark page,
  // painting an opaque white sheet behind the glass panels.
  await expect
    .poll(dockTheme, { message: "dockview theme after switching to dark" })
    .toContain("dark");

  await page.getByRole("button", { name: "Use light mode" }).click();
  await expect(page.locator("html")).not.toHaveClass(/dark/);
  await expect
    .poll(dockTheme, { message: "dockview theme after switching to light" })
    .toContain("light");
});
