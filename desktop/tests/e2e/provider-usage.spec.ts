import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

test("provider allowance stays off until the user opts in", async ({
  page,
}) => {
  await installMockBridge(page, undefined, { seedPreviewFeatures: false });
  await page.goto("/", { waitUntil: "domcontentloaded" });

  await expect(page.getByTestId("sidebar-provider-usage")).toHaveCount(0);

  await page.getByTestId("open-settings").click();
  await page.getByTestId("profile-popover-settings").click();
  await page.getByTestId("settings-nav-experimental").click();

  const experiments = page.getByTestId("settings-experimental");
  const toggle = experiments.getByTestId("feature-toggle-providerUsage");
  await expect(toggle).not.toBeChecked();
  await toggle.click();
  await expect(toggle).toBeChecked();
  await expect(page.getByTestId("sidebar-provider-usage")).toContainText(
    /Codex\s*48%/,
  );
});

test("provider allowance picker and multi-window sidebar stay provider-scoped", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });

  const indicator = page.getByTestId("sidebar-provider-usage").first();
  await expect(indicator).toContainText(/Codex\s*48%/);
  await indicator.click();
  await expect(page.getByText("Weekly · Resets")).toBeVisible();
  await expect(page.getByText("5-hour")).toBeVisible();

  await page.getByTestId("open-settings").click();
  await page.getByTestId("profile-popover-settings").click();
  await page.getByTestId("settings-nav-experimental").click();

  const experiments = page.getByTestId("settings-experimental");
  await expect(experiments.getByText("Allowance provider")).toBeVisible();
  await expect(
    experiments.getByRole("button", { name: /Auto/ }),
  ).toHaveAttribute("aria-pressed", "true");
  await expect(
    experiments.getByRole("button", { name: /Claude/ }),
  ).toBeDisabled();
  await expect(
    experiments.getByRole("button", { name: /Grok/ }),
  ).toBeDisabled();

  await experiments.getByRole("button", { name: /Codex/ }).click();
  await expect(
    experiments.getByRole("button", { name: /Codex/ }),
  ).toHaveAttribute("aria-pressed", "true");
  await page.reload({ waitUntil: "domcontentloaded" });
  await expect(
    page.getByTestId("sidebar-provider-usage").first(),
  ).toContainText(/Codex\s*48%/);
  await expect(
    page
      .getByTestId("settings-experimental")
      .getByRole("button", { name: /Codex/ }),
  ).toHaveAttribute("aria-pressed", "true");
});

test("provider allowance stays compact, theme-native, and accessible", async ({
  page,
}) => {
  await page.setViewportSize({ width: 640, height: 500 });
  await page.addInitScript(() => {
    Object.defineProperty(navigator, "platform", { get: () => "MacIntel" });
    window.localStorage.setItem("buzz-theme", "github-light");
    window.localStorage.setItem("buzz-accent-color", "#a855f7");
    window.localStorage.setItem("buzz-follow-system", "false");
    window.localStorage.setItem("buzz:text-scale", "1.5");
  });
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });

  await expect
    .poll(() =>
      page.evaluate(() => getComputedStyle(document.documentElement).fontSize),
    )
    .toBe("24px");

  const topChrome = page.getByTestId("app-top-chrome");
  const indicator = topChrome.getByTestId("sidebar-provider-usage");
  await expect(indicator).toHaveAttribute(
    "aria-label",
    /Open AI usage details\. Codex Pro: 48% left/,
  );
  await expect(indicator).toContainText(/Codex\s*48%/);

  const [indicatorBox, navRightEdge] = await Promise.all([
    indicator.boundingBox(),
    topChrome
      .locator("[data-top-chrome-nav]")
      .evaluateAll((elements) =>
        Math.max(
          ...elements.map((element) => element.getBoundingClientRect().right),
        ),
      ),
  ]);
  expect(indicatorBox).not.toBeNull();
  expect(indicatorBox?.x ?? 0).toBeGreaterThanOrEqual(navRightEdge);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBe(true);

  await indicator.focus();
  await page.keyboard.press("Enter");
  const usageDialog = page.getByRole("dialog", { name: "AI usage" });
  await expect(usageDialog).toBeVisible();
  await expect(usageDialog).toHaveAccessibleDescription(
    "Personal provider allowance",
  );
  await expect(
    usageDialog.getByRole("region", { name: "Codex Pro" }),
  ).toBeVisible();
  await expect(
    usageDialog.getByRole("progressbar", { name: "Codex: 48% remaining" }),
  ).toHaveAttribute("aria-valuetext", /Weekly; resets/);
  const dialogBox = await usageDialog.boundingBox();
  expect(dialogBox).not.toBeNull();
  expect((dialogBox?.y ?? 0) + (dialogBox?.height ?? 0)).toBeLessThanOrEqual(
    500,
  );
  await usageDialog
    .getByRole("button", { name: "Refresh Codex allowance" })
    .click();
  await expect(
    usageDialog.getByRole("status").filter({
      hasText: "Codex allowance updated.",
    }),
  ).toBeAttached();
  await page.keyboard.press("Escape");
  await expect(indicator).toBeFocused();
  await page.keyboard.press("Space");
  await expect(usageDialog).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(indicator).toBeFocused();

  await page.setViewportSize({ width: 1280, height: 900 });
  await page.getByTestId("open-settings").click();
  await page.getByTestId("profile-popover-settings").click();
  await page.getByTestId("settings-nav-appearance").click();
  const settingsSidebar = page.getByTestId("settings-sidebar");
  const settingsIndicator = settingsSidebar.getByTestId(
    "sidebar-provider-usage",
  );
  await expect(settingsIndicator).toContainText("AI usage");
  await expect(settingsIndicator).toContainText("Codex");
  const [settingsBox, sidebarBoxAtFullSize] = await Promise.all([
    settingsIndicator.boundingBox(),
    settingsSidebar.boundingBox(),
  ]);
  expect(settingsBox).not.toBeNull();
  expect(sidebarBoxAtFullSize).not.toBeNull();
  expect(
    (settingsBox?.y ?? 0) + (settingsBox?.height ?? 0),
  ).toBeLessThanOrEqual(
    (sidebarBoxAtFullSize?.y ?? 0) + (sidebarBoxAtFullSize?.height ?? 0),
  );

  const warningRing = settingsIndicator.locator("circle").nth(1);
  const warningColor = await page.evaluate(() => {
    const probe = document.createElement("span");
    probe.style.color = "var(--ui-warning)";
    document.body.append(probe);
    const color = getComputedStyle(probe).color;
    probe.remove();
    return color;
  });
  await expect
    .poll(() =>
      warningRing.evaluate((element) => getComputedStyle(element).stroke),
    )
    .toBe(warningColor);

  await page.getByTestId("appearance-mode-dark").click();
  await expect(page.locator("html")).toHaveClass(/dark/);
  const darkWarningColor = await page.evaluate(() => {
    const probe = document.createElement("span");
    probe.style.color = "var(--ui-warning)";
    document.body.append(probe);
    const color = getComputedStyle(probe).color;
    probe.remove();
    return color;
  });
  await expect
    .poll(() =>
      warningRing.evaluate((element) => getComputedStyle(element).stroke),
    )
    .toBe(darkWarningColor);

  await page.setViewportSize({ width: 800, height: 500 });
  await settingsIndicator.click();
  const settingsDialog = page.getByRole("dialog", { name: "AI usage" });
  await expect(settingsDialog).toBeVisible();
  const [settingsDialogBox, sidebarBox] = await Promise.all([
    settingsDialog.boundingBox(),
    settingsSidebar.boundingBox(),
  ]);
  expect(settingsDialogBox).not.toBeNull();
  expect(sidebarBox).not.toBeNull();
  expect(settingsDialogBox?.x ?? 0).toBeGreaterThanOrEqual(
    (sidebarBox?.x ?? 0) + (sidebarBox?.width ?? 0),
  );
  expect(
    (settingsDialogBox?.x ?? 0) + (settingsDialogBox?.width ?? 0),
  ).toBeLessThanOrEqual(800);
  expect(
    (settingsDialogBox?.y ?? 0) + (settingsDialogBox?.height ?? 0),
  ).toBeLessThanOrEqual(500);
});
