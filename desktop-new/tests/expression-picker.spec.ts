import { expect, test } from "@playwright/test";

test("explores seeded emoji and GIF picker UI", async ({ page }) => {
  await page.goto("/design/components/composer");
  const composer = page.getByRole("region", { name: "Composer playground" });
  await composer.getByRole("button", { name: "Choose emoji or GIF" }).click();
  await expect(page.getByRole("tab", { name: "Emoji" })).toBeVisible();
  await page.getByRole("textbox", { name: "Search emoji" }).fill("party");
  await expect(
    page.getByRole("button", { name: "Insert :party-parrot:" }),
  ).toBeVisible();
  const parrot = page.getByRole("button", { name: "Insert :party-parrot:" });
  await parrot.hover();
  await expect(page.getByText("party-parrot", { exact: true })).toBeVisible();
  await parrot.click();
  await expect(composer.getByRole("textbox")).toContainText(":party-parrot:");
  await composer.getByRole("button", { name: "Choose emoji or GIF" }).click();
  await page.getByRole("tab", { name: "GIFs" }).click();
  await expect(
    page.getByRole("textbox", { name: "Search GIFs" }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Choose GIF 1" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Choose GIF 1" }).click();
  await expect(page.getByRole("button", { name: "Choose GIF 1" })).toHaveCount(
    0,
  );
});
