import { expect, test } from "@playwright/test";
import { waitForAnimations } from "../../desktop/tests/helpers/animations";

test.beforeEach(async ({ page }) => {
  await page.goto("/design/components#ai-composer");
});

test("composer shares pointer and keyboard submission while preserving IME and modifiers", async ({
  page,
}) => {
  const input = page.getByRole("textbox", { name: "Prompt the assistant" });
  const send = page.getByRole("button", { name: "Send message", exact: true });
  await expect(send).toBeDisabled();
  await input.fill("   ");
  await expect(send).toBeDisabled();
  await input.fill("First line");
  await input.press("End");
  await input.press("Shift+Enter");
  await input.pressSequentially("Second line");
  await expect(input).toHaveValue("First line\nSecond line");
  for (const key of ["Control+Enter", "Meta+Enter", "Alt+Enter"]) {
    await input.press(key);
    await expect(
      page.getByRole("button", { name: "Stop response" }),
    ).toHaveCount(0);
  }
  await input.dispatchEvent("compositionstart");
  await input.press("Enter");
  await expect(page.getByRole("button", { name: "Stop response" })).toHaveCount(
    0,
  );
  await input.dispatchEvent("compositionend");
  await input.fill("Compose a useful response");
  await input.press("Enter");
  await expect(input).toHaveValue("");
  await expect(input).toHaveAttribute("readonly", "");
  await expect(
    page.getByRole("combobox", { name: "Model", exact: true }),
  ).toBeDisabled();
  await page.getByRole("button", { name: "Stop response" }).click();
  await expect(input).toBeEditable();
  await input.fill("Another prompt");
  await send.click();
  await page.getByRole("button", { name: "Finish preview" }).click();
  await expect(input).toBeEditable();
});

test("failed sends retain the draft and allow an explicit retry", async ({
  page,
}) => {
  const input = page.getByRole("textbox", { name: "Prompt the assistant" });
  await input.fill("Keep this draft when sending fails");
  await page.getByRole("button", { name: "Simulate send failure" }).click();
  await page.getByRole("button", { name: "Send message", exact: true }).click();
  await expect(page.getByRole("alert")).toHaveText(
    "Couldn’t send. Your draft is still here. Try again.",
  );
  await expect(input).toHaveValue("Keep this draft when sending fails");
  await expect(input).toHaveAttribute("aria-invalid", "true");
  await input.press("Enter");
  await expect(page.getByRole("alert")).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Stop response" }),
  ).toBeVisible();
});

test("attachments, context, model controls, and transcript preview are functional", async ({
  page,
}) => {
  const section = page.locator("#ai-composer");
  const chooserEvent = page.waitForEvent("filechooser");
  await section.getByRole("button", { name: "Add attachment" }).click();
  const chooser = await chooserEvent;
  await chooser.setFiles([
    {
      name: "notes.txt",
      mimeType: "text/plain",
      buffer: Buffer.from("Local attachment"),
    },
  ]);
  await section
    .getByRole("button", { name: "Remove attachment notes.txt" })
    .click();
  await expect(section.getByText("notes.txt", { exact: true })).toHaveCount(0);
  await section.getByRole("button", { name: "Remove annotation" }).click();
  await expect(section.getByText("1 annotation", { exact: true })).toHaveCount(
    0,
  );
  await section.getByRole("combobox", { name: "Model", exact: true }).click();
  await page.getByRole("option", { name: "Fast", exact: true }).click();
  await section.getByRole("combobox", { name: "Action policy" }).click();
  await page.getByRole("option", { name: "Read only", exact: true }).click();
  await section
    .getByRole("button", { name: "Insert demo voice transcript" })
    .click();
  await expect(section.getByRole("status")).toContainText(
    "The microphone was not accessed.",
  );
  await section
    .getByRole("button", { name: "Send message", exact: true })
    .click();
  await expect(section.getByRole("status")).toContainText(
    "fast · read · 0 attachments · 0 annotations",
  );
  await section.getByRole("button", { name: "Stop response" }).click();
  await section.locator('input[type="file"]').setInputFiles(
    Array.from({ length: 6 }, (_, i) => ({
      name: `file-${i}.txt`,
      mimeType: "text/plain",
      buffer: Buffer.from("demo"),
    })),
  );
  await expect(section.getByRole("alert")).toContainText(
    "Choose up to 5 files",
  );
  await expect(
    section.getByRole("button", { name: /^Remove attachment/ }),
  ).toHaveCount(0);
});

test("card header is compact and composer fits narrow layouts", async ({
  page,
}, testInfo) => {
  await expect(page.locator("#card .bui-card-header")).toHaveCSS(
    "row-gap",
    "8px",
  );
  for (const width of [390, 884, 1440]) {
    await page.setViewportSize({ width, height: 1000 });
    const composer = page.locator("#ai-composer .bui-composer");
    await expect(composer).toBeVisible();
    await expect(composer.locator(".bui-composer-footer")).toHaveCSS(
      "height",
      "40px",
    );
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await composer.scrollIntoViewIfNeeded();
    await waitForAnimations(page);
    await composer.screenshot({
      path: testInfo.outputPath(`composer-${width}.png`),
    });
  }
  await page.getByRole("button", { name: "Switch to dark mode" }).click();
  await waitForAnimations(page);
  await page
    .locator("#ai-composer .bui-composer")
    .screenshot({ path: testInfo.outputPath("composer-dark.png") });
});
