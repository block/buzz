import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { waitForAnimations } from "../../desktop/tests/helpers/animations";

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() =>
    localStorage.setItem("buzz-next-color-scheme", "light"),
  );
});
for (const mode of ["light", "dark"] as const) {
  test(`catalog renders all examples accessibly in ${mode}`, async ({
    page,
  }) => {
    const errors: string[] = [];
    const external: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    page.on("request", (request) => {
      if (!new URL(request.url()).hostname.match(/^(127\.0\.0\.1|localhost)$/))
        external.push(request.url());
    });
    await page.goto("/design/components");
    if (mode === "dark")
      await page.getByRole("button", { name: "Switch to dark mode" }).click();
    await expect(page.locator(".catalog-entry")).toHaveCount(55);
    await expect(
      page.getByRole("button", { name: "Saving", exact: true }),
    ).toBeDisabled();
    await expect(
      page.getByRole("button", { name: "Saving", exact: true }),
    ).toHaveAccessibleName("Saving");
    const results = await new AxeBuilder({ page })
      .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"])
      .disableRules(["color-contrast"])
      .analyze();
    // Text contrast is governed by the repository's APCA audit, which runs separately.
    expect(results.violations).toEqual([]);
    expect(errors).toEqual([]);
    expect(external).toEqual([]);
  });
}
test("checkbox and switch respond to keyboard and keep one accessible label", async ({
  page,
}) => {
  await page.goto("/design/components");
  const checkbox = page.getByRole("checkbox", {
    name: "Notify me about replies",
    exact: true,
  });
  await expect(checkbox).toHaveCount(1);
  await expect(checkbox).toBeChecked();
  await checkbox.press("Space");
  await expect(checkbox).not.toBeChecked();
  const toggle = page.getByRole("switch", {
    name: "Desktop notifications",
    exact: true,
  });
  await toggle.press("Space");
  await expect(toggle).not.toBeChecked();
});
test("dialog traps focus, dismisses with Escape, and returns focus to its trigger", async ({
  page,
}) => {
  await page.goto("/design/components");
  const trigger = page.getByRole("button", {
    name: "Edit project",
    exact: true,
  });
  await trigger.click();
  const dialog = page.getByRole("dialog", {
    name: "A place to build together",
  });
  await expect(dialog).toBeVisible();
  await page.keyboard.press("Tab");
  await expect(dialog.getByRole("button", { name: "Done" })).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(dialog.getByRole("button", { name: "Done" })).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  await expect(trigger).toBeFocused();
});
test("select and command use keyboard selection", async ({ page }) => {
  await page.goto("/design/components");
  const select = page.getByRole("combobox", { name: "Digest frequency" });
  await select.click();
  await page.getByRole("option", { name: "Weekly digest" }).click();
  await expect(select).toContainText("Weekly digest");
  const command = page.getByRole("combobox", { name: "Find a command" });
  await command.fill("Search");
  await command.press("ArrowDown");
  await command.press("Enter");
  await expect(
    page.getByText("Selected: search", { exact: true }),
  ).toBeVisible();
});
test("form validates and accepts a valid entry; textarea accepts multiline content", async ({
  page,
}) => {
  await page.goto("/design/components");
  await page.getByRole("button", { name: "Invite member" }).click();
  await expect(
    page.getByText("Invitation prepared in this demo.", { exact: true }),
  ).toBeHidden();
  await page
    .getByRole("textbox", { name: "Email", exact: true })
    .fill("builder@example.org");
  await page.getByRole("button", { name: "Invite member" }).click();
  await expect(
    page.getByText("Invitation prepared in this demo.", { exact: true }),
  ).toBeVisible();
  const textarea = page.getByRole("textbox", { name: "Project description" });
  await textarea.fill("First line\nSecond line");
  await expect(textarea).toHaveValue("First line\nSecond line");
});
test("tabs, carousel, and resizable panels preserve keyboard paths", async ({
  page,
}) => {
  await page.goto("/design/components");
  await page
    .getByRole("tab", { name: "Overview", exact: true })
    .press("ArrowRight");
  await expect(
    page.getByRole("tab", { name: "Activity", exact: true }),
  ).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(
    page.getByRole("tab", { name: "Activity", exact: true }),
  ).toHaveAttribute("aria-selected", "true");
  await page.getByRole("button", { name: "Next slide" }).click();
  await expect(page.getByRole("group", { name: "2 of 3" })).toContainText(
    "Compose primitives",
  );
  const handle = page.getByRole("separator", { name: "Resize project list" });
  const before = await handle.getAttribute("aria-valuenow");
  await handle.press("ArrowRight");
  await expect(handle).not.toHaveAttribute("aria-valuenow", before ?? "");
});
test("filtering has an explicit empty-state recovery", async ({ page }) => {
  await page.goto("/design/components");
  await page
    .getByRole("searchbox", { name: "Find a component" })
    .fill("nothing-matches-this");
  await expect(page.locator(".catalog-entry")).toHaveCount(0);
  await page.getByRole("button", { name: "Clear filters" }).click();
  await expect(page.locator(".catalog-entry")).toHaveCount(55);
});
test("generated actions require a click, disable during streaming, and recover after invalid data", async ({
  page,
}) => {
  await page.goto("/design/compositions");
  const inspect = page.getByRole("button", { name: "Inspect changes" });
  await expect(
    page.getByText("Actions in these examples update this preview only."),
  ).toBeVisible();
  await page.getByRole("button", { name: "Preview streaming" }).click();
  await expect(inspect).toBeDisabled();
  await page.getByRole("button", { name: "Finish streaming" }).click();
  await inspect.click();
  await expect(
    page.getByText("Inspect changes requested", { exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Preview invalid response" }).click();
  await expect(
    page.getByText("This response could not be displayed"),
  ).toBeVisible();
  await page.getByRole("button", { name: "Try again" }).click();
  await expect(inspect).toBeEnabled();
});
test("responsive overview and compositions have no horizontal page overflow", async ({
  page,
}) => {
  for (const width of [390, 768, 1440]) {
    await page.setViewportSize({ width, height: 1000 });
    for (const route of [
      "/design",
      "/design/components",
      "/design/compositions",
      "/design/open-source",
    ]) {
      await page.goto(route);
      await expect(page.locator("h1")).toBeVisible();
      expect(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= window.innerWidth,
        ),
      ).toBe(true);
    }
  }
});
test("reduced motion removes press movement and loading animation", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/design");
  await expect(page.locator(".bui-loading")).toHaveCSS(
    "animation-name",
    "none",
  );
  expect(
    await page
      .locator("html")
      .evaluate((element) =>
        getComputedStyle(element).getPropertyValue("--press-scale").trim(),
      ),
  ).toBe("1");
});
test("capture the complete overview and composed examples", async ({
  page,
}, testInfo) => {
  await page.setViewportSize({ width: 1440, height: 1100 });
  await page.goto("/design");
  await page.evaluate(() => document.fonts.ready);
  await waitForAnimations(page);
  await page.screenshot({
    path: testInfo.outputPath("overview-light.png"),
    fullPage: true,
  });
  await page.getByRole("button", { name: "Switch to dark mode" }).click();
  await waitForAnimations(page);
  await page.screenshot({
    path: testInfo.outputPath("overview-dark.png"),
    fullPage: true,
  });
  await page.goto("/design/compositions");
  await waitForAnimations(page);
  await page.screenshot({
    path: testInfo.outputPath("compositions-dark.png"),
    fullPage: true,
  });
});

test("all modal surfaces expose names, close paths, and focus return", async ({
  page,
}) => {
  await page.goto("/design/components");
  for (const [triggerName, role, title, closeName] of [
    ["Project details", "dialog", "Design system", "Close details"],
    ["Open drawer", "dialog", "Take the next step", "Continue building"],
    ["Delete draft", "alertdialog", "Delete this draft?", "Keep draft"],
    ["Share options", "dialog", "Share your progress", "Got it"],
  ] as const) {
    const trigger = page.getByRole("button", {
      name: triggerName,
      exact: true,
    });
    await trigger.click();
    const surface = page.getByRole(role, { name: title, exact: true });
    await expect(surface).toBeVisible();
    await surface.getByRole("button", { name: closeName, exact: true }).click();
    await expect(surface).toBeHidden();
    await expect(trigger).toBeFocused();
  }
  await page.getByRole("button", { name: "Show notification" }).click();
  await expect(page.getByText("Changes saved", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Dismiss notification" }).click();
  await expect(page.getByText("Changes saved", { exact: true })).toBeHidden();
});

test("every foundation page renders its public vocabulary without errors", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  for (const route of [
    "color",
    "color/table",
    "typography",
    "spacing",
    "radius",
    "elevation",
    "glass",
    "motion",
    "vocabulary",
    "growth",
  ]) {
    await page.goto(`/design/${route}`);
    await expect(page.locator("h1")).toBeVisible();
    await expect(
      page.getByText("Something went wrong", { exact: true }),
    ).toBeHidden();
  }
  expect(errors).toEqual([]);
});

test("styled navigation preserves link semantics and opens the component catalog", async ({
  page,
}) => {
  await page.goto("/design");
  await page
    .getByRole("link", { name: "Explore 55 examples", exact: true })
    .click();
  await expect(page).toHaveURL("/design/components");
  await expect(page.locator(".catalog-entry")).toHaveCount(55);
});
