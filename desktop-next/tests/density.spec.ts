import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";
import { waitForAnimations } from "../../desktop/tests/helpers/animations";

test("density changes geometry throughout without changing type or losing a draft", async ({
  page,
  context,
}) => {
  await page.goto("/design/components");
  const normal = page.getByRole("button", { name: "Normal", exact: true });
  const compact = page.getByRole("button", { name: "Compact", exact: true });
  const button = page.getByRole("button", { name: "Continue", exact: true });
  const input = page.getByRole("textbox", {
    name: "Project name",
    exact: true,
  });
  const draft = page.getByRole("textbox", { name: "Prompt the assistant" });
  await expect(normal).toHaveAttribute("aria-pressed", "true");
  await expect(button).toHaveCSS("height", "40px");
  await expect(input).toHaveCSS("font-size", "14px");
  await draft.fill("Keep my draft while changing density");
  const anotherTab = await context.newPage();
  await anotherTab.goto("/design/glass");

  await compact.focus();
  await compact.press("Space");
  await expect(compact).toHaveAttribute("aria-pressed", "true");
  await expect(page.locator("html")).toHaveAttribute("data-density", "compact");
  await expect(anotherTab.locator("html")).toHaveAttribute(
    "data-density",
    "compact",
  );
  await expect(button).toHaveCSS("height", "32px");
  await expect(input).toHaveCSS("height", "32px");
  await expect(input).toHaveCSS("font-size", "14px");
  await expect(draft).toHaveValue("Keep my draft while changing density");
  await expect(page.locator("#card .bui-card")).toHaveCSS("padding", "16px");
  await expect(page.locator("#table td").first()).toHaveCSS("padding", "8px");
  await expect(page.locator(".catalog-grid")).toHaveCSS("row-gap", "32px");
  await expect(page.locator("main header").first()).toHaveCSS(
    "margin-bottom",
    "42px",
  );
  await expect(draft).toHaveCSS("min-height", "64px");

  // Portals are outside the catalog but inherit the same root preference.
  await page.getByRole("button", { name: "Edit project", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCSS("padding", "16px");
  await expect(
    page.getByRole("button", { name: "Done", exact: true }),
  ).toHaveCSS("height", "32px");
  await page.keyboard.press("Escape");
  await page.getByRole("combobox", { name: "Digest frequency" }).click();
  await expect(page.getByRole("option").first()).toHaveCSS(
    "padding",
    "4px 8px",
  );
  await page.keyboard.press("Escape");

  await page.getByRole("button", { name: "Switch to dark mode" }).click();
  await expect(page.locator("html")).toHaveClass(/dark/);
  await expect(button).toHaveCSS("height", "32px");
  await page.reload();
  await expect(compact).toHaveAttribute("aria-pressed", "true");
  await expect(page.locator("html")).toHaveClass(/dark/);
  await page
    .getByRole("navigation", { name: "Design system", exact: true })
    .getByRole("link", { name: "Spacing", exact: true })
    .click();
  const token = page
    .locator("dl > div")
    .filter({ has: page.getByText("control-md", { exact: true }) });
  await expect(token).toContainText("2rem");
  await page
    .getByRole("navigation", { name: "Design system", exact: true })
    .getByRole("link", { name: "Compositions", exact: true })
    .click();
  await expect(page.locator(".bui-card").first()).toHaveCSS("padding", "16px");
  await normal.click();
  await expect(page.locator(".bui-card").first()).toHaveCSS("padding", "24px");
  await expect(anotherTab.locator("html")).toHaveAttribute(
    "data-density",
    "normal",
  );
  await expect(page.locator("html")).toHaveClass(/dark/);
  await anotherTab.close();
});

test("compact works at narrow and wide sizes in both themes, including Glass tabs", async ({
  page,
}, testInfo) => {
  await page.addInitScript(() =>
    localStorage.setItem("buzz-next-density", "compact"),
  );
  await page.goto("/design/components");
  for (const width of [390, 884, 1440]) {
    await page.setViewportSize({ width, height: 960 });
    for (const scheme of ["light", "dark"] as const) {
      const changeTheme = page.getByRole("button", {
        name: `Switch to ${scheme} mode`,
      });
      if (await changeTheme.count()) await changeTheme.click();
      await expect(page.locator(".catalog-entry")).toHaveCount(55);
      expect(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= innerWidth,
        ),
      ).toBe(true);
      const composer = page.locator("#ai-composer .bui-composer");
      expect(
        await composer.evaluate((el) => el.scrollWidth <= el.clientWidth),
      ).toBe(true);
      await composer.scrollIntoViewIfNeeded();
      await waitForAnimations(page);
      await composer.screenshot({
        path: testInfo.outputPath(`compact-composer-${width}-${scheme}.png`),
      });
    }
  }
  const axe = await new AxeBuilder({ page })
    .disableRules(["color-contrast"])
    .analyze();
  expect(axe.violations).toEqual([]);
  await page.goto("/design/glass");
  const tabs = page.getByRole("tablist");
  await tabs.getByRole("tab", { name: "Me", exact: true }).click();
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("Enter");
  await expect(
    tabs.getByRole("tab", { name: "Messages", exact: true }),
  ).toHaveAttribute("aria-selected", "true");
  await expect(tabs.getByRole("tab").first()).toHaveCSS("padding-left", "12px");
  await page.getByRole("button", { name: "Normal", exact: true }).click();
  await expect(tabs.getByRole("tab").first()).toHaveCSS("padding-left", "20px");
  await expect(
    tabs.getByRole("tab", { name: "Messages", exact: true }),
  ).toHaveAttribute("aria-selected", "true");
});

test("invalid saved density falls back and a failed save stays usable with retry", async ({
  page,
}) => {
  await page.addInitScript(() => {
    localStorage.setItem("buzz-next-density", "invalid");
    const original = Storage.prototype.setItem;
    let shouldFail = true;
    Storage.prototype.setItem = function (key, value) {
      if (key === "buzz-next-density" && shouldFail) {
        shouldFail = false;
        throw new DOMException("Storage unavailable", "QuotaExceededError");
      }
      return original.call(this, key, value);
    };
  });
  await page.goto("/design/components");
  await expect(
    page.getByRole("button", { name: "Normal", exact: true }),
  ).toHaveAttribute("aria-pressed", "true");
  const compact = page.getByRole("button", { name: "Compact", exact: true });
  await compact.click();
  await expect(page.getByText(/Your browser couldn’t save it/)).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Continue", exact: true }),
  ).toHaveCSS("height", "32px");
  await compact.click();
  await expect(page.getByText(/Your browser couldn’t save it/)).toHaveCount(0);
  expect(
    await page.evaluate(() => localStorage.getItem("buzz-next-density")),
  ).toBe("compact");
});
