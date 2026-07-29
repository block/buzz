import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

// Navigator rows open on the first click while keyboard ↑/↓ stays
// preview-only (Enter opens), and the keyboard-shortcuts pane replaces the
// visible key hints — opened with `?` in an empty composer or from the
// command palette.

async function openDevMode(page: import("@playwright/test").Page) {
  await installMockBridge(page);
  await page.addInitScript(() => {
    localStorage.setItem("buzz.displayStyle", "developer");
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("dev-mode-composer").waitFor();
}

test("clicking a navigator channel opens it immediately", async ({ page }) => {
  await openDevMode(page);

  await page
    .getByTestId("dev-mode-channel-navigator")
    .getByText("# general", { exact: true })
    .click();

  await expect(page.getByTestId("dev-mode-topbar-channel")).toContainText(
    "general",
  );
  // Open, not previewing: no preview strip, and the composer targets the
  // channel.
  await expect(page.getByText("preview", { exact: true })).toHaveCount(0);
  await expect(page.getByTestId("dev-mode-composer")).toHaveAttribute(
    "placeholder",
    /Message # general/,
  );
});

test("keyboard ArrowUp previews; Enter is required to open", async ({
  page,
}) => {
  await openDevMode(page);

  const composer = page.getByTestId("dev-mode-composer");
  await composer.focus();
  await page.keyboard.press("ArrowUp");

  await expect(page.getByText("preview", { exact: true })).toBeVisible();
  await expect(page.getByTestId("dev-mode-topbar-channel")).toBeVisible();

  await page.keyboard.press("Enter");
  await expect(page.getByText("preview", { exact: true })).toHaveCount(0);
  await expect(composer).toHaveAttribute("placeholder", /Message # /);
});

test("`?` in an empty composer opens the shortcuts pane", async ({ page }) => {
  await openDevMode(page);

  await page.getByTestId("dev-mode-composer").focus();
  await page.keyboard.press("?");

  const overlay = page.getByTestId("dev-mode-shortcuts-overlay");
  await expect(overlay).toBeVisible();
  await expect(overlay).toContainText("command palette");

  await page.keyboard.press("Escape");
  await expect(overlay).toHaveCount(0);
  // Focus returns to the composer so typing resumes immediately.
  await expect(page.getByTestId("dev-mode-composer")).toBeFocused();
});

test("`?` while text is present types normally", async ({ page }) => {
  await openDevMode(page);

  const composer = page.getByTestId("dev-mode-composer");
  await composer.focus();
  await composer.pressSequentially("what?");

  await expect(page.getByTestId("dev-mode-shortcuts-overlay")).toHaveCount(0);
  await expect(composer).toHaveValue("what?");
});

test("palette's keyboard-shortcuts entry opens the pane", async ({ page }) => {
  await openDevMode(page);

  await page.keyboard.press("Meta+k");
  await page
    .getByTestId("dev-mode-palette-input")
    .pressSequentially("keyboard short");
  await page.keyboard.press("Enter");

  await expect(page.getByTestId("dev-mode-shortcuts-overlay")).toBeVisible();
});
