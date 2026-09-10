import { expect, test } from "@playwright/test";
import { waitForAnimations } from "../../desktop/tests/helpers/animations";

test("previews all six Figma phrases without changing another composer", async ({
  page,
}) => {
  await page.goto("/design/components/composer");
  const empty = page.getByRole("region", {
    name: "Composer playground",
    exact: true,
  });
  const choices = [
    ["Channel · New message", "Send a message in #buzz-design"],
    ["Channel · Reply", "Reply in #buzz-design"],
    ["Session · New session", "Start a new session"],
    ["Session · Reply", "Reply in this session"],
    ["Direct message · New message", "Start a new direct message"],
    ["Direct message · Reply", "Reply in this direct message"],
  ];
  for (const [label, copy] of choices) {
    await empty.getByRole("combobox", { name: "Preview context" }).click();
    await expect(page.getByRole("option")).toHaveCount(6);
    await page.getByRole("option", { name: label, exact: true }).click();
    await expect(
      empty.getByRole("combobox", { name: "Preview context" }),
    ).toContainText(label);
    await expect(empty.getByRole("textbox")).toHaveAttribute(
      "aria-label",
      copy,
    );
    await expect(
      empty.getByRole("button", { name: "Record voice note" }),
    ).toBeVisible();
  }
  const second = page.getByRole("region", {
    name: "Unavailable",
    exact: true,
  });
  await expect(second.getByRole("textbox")).toHaveAttribute(
    "aria-label",
    "Reply in this session",
  );
  const trigger = empty.getByRole("combobox", { name: "Preview context" });
  await trigger.focus();
  await trigger.press("Enter");
  await expect(page.getByRole("listbox")).toBeVisible();
  await waitForAnimations(page);
  await page.keyboard.press("Home");
  await page.keyboard.press("Enter");
  await expect(empty.getByRole("textbox")).toHaveAttribute(
    "aria-label",
    "Send a message in #buzz-design",
  );
  await expect(trigger).toBeFocused();
  await trigger.press("Enter");
  await page.keyboard.press("Escape");
  await expect(trigger).toBeFocused();
  await page.setViewportSize({ width: 390, height: 850 });
  await trigger.click();
  await expect(page.getByRole("option")).toHaveCount(6);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
});
