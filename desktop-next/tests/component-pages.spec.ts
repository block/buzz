import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";
import { waitForAnimations } from "../../desktop/tests/helpers/animations";

test("every catalog entry links to its own focused component page", async ({
  page,
}) => {
  await page.goto("/design/components");
  const entries = await page.locator(".catalog-entry").evaluateAll((links) =>
    links.map((link) => ({
      name: link.querySelector("h2")?.textContent ?? "",
      href: link.querySelector(".catalog-eyebrow")?.getAttribute("href") ?? "",
    })),
  );
  expect(entries).toHaveLength(55);
  await expect(page.getByText("Explore states →", { exact: true })).toHaveCount(
    0,
  );
  await expect(page.locator(".catalog-grid .catalog-source")).toHaveCount(0);
  for (const eyebrow of await page.locator(".catalog-eyebrow").all()) {
    await expect(eyebrow).toContainText("→");
  }
  const nav = page.getByRole("navigation", {
    name: "Design system",
    exact: true,
  });
  await expect(nav.locator('a[href^="/design/components/"]')).toHaveCount(55);
  expect(new Set(entries.map((entry) => entry.href)).size).toBe(55);
  for (const entry of entries) {
    await page.goto(entry.href);
    await expect(
      page.getByRole("heading", { level: 1, name: entry.name, exact: true }),
    ).toBeVisible();
    await expect(page.locator(".component-playground")).toHaveCount(1);
    await expect(page.locator(".catalog-entry")).toHaveCount(0);
    expect(
      await page
        .getByRole("complementary", { name: "States to explore" })
        .getByRole("listitem")
        .count(),
    ).toBeGreaterThan(0);
    await expect(page).toHaveTitle(`${entry.name} — Buzz Design System`);
  }
});

test("catalog navigation, picker, previous/next, reset, and invalid links work", async ({
  page,
}) => {
  await page.goto("/design/components");
  await page.getByRole("button", { name: "Compact", exact: true }).click();
  await page.getByRole("link", { name: "Inputs: Input", exact: true }).click();
  await expect(page).toHaveURL(/\/design\/components\/input$/);
  await expect(page.getByRole("heading", { level: 1 })).toBeFocused();
  const input = page.getByRole("textbox", {
    name: "Project name",
    exact: true,
  });
  await input.fill("A temporary draft");
  await expect(input).toHaveCSS("height", "32px");
  await page.getByRole("button", { name: "Reset example" }).click();
  await expect(input).toHaveValue("");
  const nav = page.getByRole("navigation", {
    name: "Design system",
    exact: true,
  });
  await expect(
    nav.getByRole("link", { name: "Input", exact: true }),
  ).toHaveAttribute("aria-current", "page");
  await expect(
    nav.getByRole("link", { name: "All components", exact: true }),
  ).not.toHaveAttribute("aria-current", "page");
  await nav.getByRole("link", { name: "Resizable", exact: true }).click();
  await expect(page).toHaveURL(/\/resizable$/);
  await expect(
    page.getByRole("button", { name: "Compact", exact: true }),
  ).toBeInViewport();
  await nav.getByRole("link", { name: "Input", exact: true }).click();
  await page
    .getByRole("link", { name: "Next: Textarea", exact: false })
    .click();
  await expect(page).toHaveURL(/\/textarea$/);
  await page
    .getByRole("link", { name: "Previous: Input", exact: false })
    .click();
  await expect(page).toHaveURL(/\/input$/);
  await page.getByRole("combobox", { name: "Jump to component" }).click();
  await page.getByRole("option", { name: "Dialog", exact: true }).click();
  await expect(page).toHaveURL(/\/dialog$/);
  await page.getByRole("button", { name: "Edit project", exact: true }).click();
  await expect(page.getByRole("dialog")).toBeVisible();
  await expect(page.getByRole("dialog")).toHaveCSS("padding", "16px");
  await page.keyboard.press("Escape");
  await page.reload();
  await expect(
    page.getByRole("heading", { level: 1, name: "Dialog" }),
  ).toBeVisible();
  await page
    .getByRole("link", { name: "All components", exact: false })
    .filter({ hasText: "←" })
    .click();
  await expect(page).toHaveURL(/\/design\/components#dialog$/);
  await expect(page.locator("#dialog")).toBeInViewport();
  await page.goto("/design/components/not-a-component");
  await expect(
    page.getByRole("heading", { name: "Component not found" }),
  ).toBeVisible();
  await page.getByRole("link", { name: "Browse all components" }).click();
  await expect(page.locator(".catalog-entry")).toHaveCount(55);
});

test("state samples use real validation, disabled, selection, and reset behavior", async ({
  page,
}) => {
  await page.goto("/design/components/input");
  const invalid = page.getByRole("textbox", { name: "Invalid", exact: true });
  await expect(invalid).toHaveAttribute("aria-invalid", "true");
  await expect(
    page.getByText("Choose a different project name."),
  ).toBeVisible();
  await expect(
    page.getByRole("textbox", { name: "Disabled", exact: true }),
  ).toBeDisabled();
  await expect(
    page.getByRole("textbox", { name: "Read-only", exact: true }),
  ).not.toBeEditable();
  await page.goto("/design/components/checkbox");
  const mixed = page.getByRole("checkbox", {
    name: "Indeterminate",
    exact: true,
  });
  await expect(mixed).toHaveAttribute("aria-checked", "mixed");
  await mixed.focus();
  await mixed.press("Space");
  await expect(mixed).toBeChecked();
  await expect(mixed).not.toHaveAttribute("aria-checked", "mixed");
  await page.getByRole("button", { name: "Reset example" }).click();
  await expect(mixed).toHaveAttribute("aria-checked", "mixed");
  await page.goto("/design/components/tabs");
  const tabs = page.getByRole("tablist", { name: "glass example" });
  await expect(tabs.getByRole("tab", { name: "Files" })).toHaveAttribute(
    "aria-disabled",
    "true",
  );
  await tabs.getByRole("tab", { name: "Activity" }).click();
  await expect(tabs.getByRole("tab", { name: "Activity" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await page.getByRole("button", { name: "Reset example" }).click();
  await expect(tabs.getByRole("tab", { name: "Overview" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
});

test("focused pages remain readable and accessible in both themes and densities", async ({
  page,
}, testInfo) => {
  for (const component of ["input", "ai-composer", "tabs"]) {
    await page.goto(`/design/components/${component}`);
    for (const width of [390, 884, 1440]) {
      await page.setViewportSize({ width, height: 960 });
      for (const density of ["Normal", "Compact"]) {
        await page.getByRole("button", { name: density, exact: true }).click();
        expect(
          await page.evaluate(
            () => document.documentElement.scrollWidth <= innerWidth,
          ),
        ).toBe(true);
      }
    }
    for (const theme of ["light", "dark"]) {
      const toggle = page.getByRole("button", {
        name: `Switch to ${theme} mode`,
      });
      if (await toggle.count()) await toggle.click();
      const result = await new AxeBuilder({ page })
        .disableRules(["color-contrast"])
        .analyze();
      expect(result.violations).toEqual([]);
      await page.getByRole("heading", { level: 1 }).scrollIntoViewIfNeeded();
      await waitForAnimations(page);
      await page.screenshot({
        path: testInfo.outputPath(`${component}-${theme}.png`),
      });
    }
  }
});
