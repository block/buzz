import { expect, test } from "@playwright/test";
import { waitForAnimations } from "../../desktop/tests/helpers/animations";

test.beforeEach(async ({ page }) => {
  await page.goto("/design/components");
});

test("command results have no empty-state spacer and announce recovery", async ({
  page,
}) => {
  const input = page.getByRole("combobox", { name: "Find a command" });
  await input.fill("Search");
  const option = page.getByRole("option").first();
  await expect(option).toBeVisible();
  const popup = page.locator(".bui-popup");
  const liveRegion = popup.getByRole("status");
  await expect(liveRegion).toBeAttached();
  expect(
    await liveRegion.evaluate((node) => node.getBoundingClientRect().height),
  ).toBe(0);
  await waitForAnimations(page);
  const inset = await option.evaluate((node) => {
    const popup = node.closest(".bui-popup");
    if (!popup) throw new Error("Option has no popup");
    return node.getBoundingClientRect().top - popup.getBoundingClientRect().top;
  });
  expect(inset).toBeLessThanOrEqual(10);
  await input.fill("no-such-command");
  await expect(liveRegion).toHaveText("No matching commands");
  await expect(liveRegion).toBeVisible();
  await input.fill("Search");
  await expect(option).toBeVisible();
  expect(
    await liveRegion.evaluate((node) => node.getBoundingClientRect().height),
  ).toBe(0);
});

test("dialog groups its heading and accordion rows stay compact", async ({
  page,
}) => {
  await page.getByRole("button", { name: "Edit project", exact: true }).click();
  const dialog = page.getByRole("dialog", {
    name: "A place to build together",
  });
  const gaps = await dialog.evaluate((node) => {
    const title = node.querySelector(".bui-title");
    const description = node.querySelector(".bui-description");
    const button = node.querySelector("button");
    if (!title || !description || !button)
      throw new Error("Missing dialog content");
    return [
      description.getBoundingClientRect().top -
        title.getBoundingClientRect().bottom,
      button.getBoundingClientRect().top -
        description.getBoundingClientRect().bottom,
    ];
  });
  expect(gaps).toEqual([16, 24]);
  await page.keyboard.press("Escape");
  const trigger = page.getByRole("button", { name: "One shared vocabulary" });
  await trigger.click();
  const accordion = page.locator("#accordion .bui-accordion");
  await expect(accordion).toHaveCSS("row-gap", "0px");
  await expect(accordion.locator(".bui-accordion-panel")).toHaveCSS(
    "padding-top",
    "0px",
  );
  expect(
    await trigger.evaluate((node) => node.getBoundingClientRect().height),
  ).toBe(40);
  await trigger.press("Space");
  await expect(trigger).toHaveAttribute("aria-expanded", "false");
});

test("toast and tab transitions animate actual state changes", async ({
  page,
}) => {
  // Stretch the existing transitions so frames can be inspected deterministically.
  // This does not add transition properties: removing production motion fails this test.
  await page.addStyleTag({
    content:
      ".bui-toast, .bui-tab-indicator { transition-duration: 2s !important; }",
  });
  await page.getByRole("button", { name: "Show notification" }).click();
  const toast = page.locator(".bui-toast");
  await expect
    .poll(() =>
      toast.evaluate((node) =>
        node
          .getAnimations()
          .map((a) => (a as CSSTransition).transitionProperty),
      ),
    )
    .toEqual(expect.arrayContaining(["opacity", "transform"]));
  await toast.evaluate((node) =>
    node.getAnimations().forEach((a) => {
      a.finish();
    }),
  );
  await toast.getByRole("button", { name: "Dismiss notification" }).click();
  await expect(toast).toHaveAttribute("data-ending-style", "");
  await expect
    .poll(() =>
      toast.evaluate((node) =>
        node
          .getAnimations()
          .map((a) => (a as CSSTransition).transitionProperty),
      ),
    )
    .toContain("transform");
  await toast.evaluate((node) =>
    node.getAnimations().forEach((a) => {
      a.finish();
    }),
  );
  await expect(toast).toBeHidden();
  const indicator = page.locator("#tabs .bui-tab-indicator");
  await page.getByRole("tab", { name: "Files", exact: true }).click();
  await expect
    .poll(() =>
      indicator.evaluate((node) =>
        node
          .getAnimations()
          .map((a) => (a as CSSTransition).transitionProperty),
      ),
    )
    .toContain("clip-path");
  // Reverse before completion: the indicator must retarget to the new selection.
  await page.getByRole("tab", { name: "Activity", exact: true }).click();
  await indicator.evaluate((node) =>
    node.getAnimations().forEach((a) => {
      a.finish();
    }),
  );
  await expect(page.getByRole("tabpanel")).toHaveText(
    "Three tasks completed today.",
  );
  const left = await indicator.evaluate((node) =>
    parseFloat(getComputedStyle(node).getPropertyValue("--active-tab-left")),
  );
  expect(left).toBeGreaterThan(4);
});

test("reduced motion preserves selection and immediate notification dismissal", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  const indicator = page.locator("#tabs .bui-tab-indicator");
  await page.getByRole("tab", { name: "Files", exact: true }).click();
  await expect(page.getByRole("tabpanel")).toHaveText(
    "Your project files live here.",
  );
  expect(await indicator.evaluate((node) => node.getAnimations().length)).toBe(
    0,
  );
  await page.getByRole("button", { name: "Show notification" }).click();
  const toast = page.locator(".bui-toast");
  await expect(toast).toHaveCSS("transition-duration", "0s, 0s");
  expect(await toast.evaluate((node) => node.getAnimations().length)).toBe(0);
  await toast.getByRole("button", { name: "Dismiss notification" }).click();
  await expect(toast).toBeHidden();
});
