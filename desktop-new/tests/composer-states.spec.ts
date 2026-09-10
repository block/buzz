import { expect, test, type Locator } from "@playwright/test";

async function bounds(locator: Locator) {
  const box = await locator.boundingBox();
  if (!box) throw new Error("Expected a rendered specimen");
  return box;
}

const path = "/design/components/composer";

test("exposes real drafts, bounded growth, formatting, and send recovery", async ({
  page,
}) => {
  await page.goto(path);
  const empty = page.getByRole("region", {
    name: "Composer playground",
    exact: true,
  });
  const input = empty.getByRole("textbox");
  // Tiptap uses a contenteditable textbox; assertions below bind its visible document.
  await expect(
    empty.getByRole("button", { name: "Send message" }),
  ).toBeDisabled();
  const small = await bounds(input);
  await input.fill("First\nSecond\nThird\nFourth");
  expect((await bounds(input)).height).toBeGreaterThan(small.height);
  await input.fill(
    Array.from({ length: 60 }, (_, i) => `Line ${i}`).join("\n"),
  );
  expect((await bounds(input)).height).toBeLessThanOrEqual(161);
  await expect(
    empty.getByRole("button", { name: "Send message" }),
  ).toBeEnabled();
  await input.fill("");
  expect((await bounds(input)).height).toBeCloseTo(small.height, 0);
  await input.fill("Format this text");
  await input.press("Meta+a");
  await empty.getByRole("button", { name: "Toggle formatting" }).click();
  await expect(empty.getByRole("group", { name: "Formatting" })).toBeVisible();
  await expect(
    empty.getByRole("button", { name: "Bold", exact: true }),
  ).toBeEnabled();
  await empty.getByRole("button", { name: "Bold", exact: true }).click();
  await expect(input.locator("strong")).toHaveText("Format this text");
  await expect(
    empty.getByRole("button", { name: "Bold", exact: true }),
  ).toHaveAttribute("aria-pressed", "true");
  await empty.getByRole("button", { name: "Close formatting" }).click();
  await expect(
    empty.getByRole("button", { name: "Attach file" }),
  ).toBeVisible();
  const send = page.getByRole("region", {
    name: "Sending and failed",
    exact: true,
  });
  await send.getByRole("button", { name: "Send message", exact: true }).click();
  await expect(send.getByRole("status")).toHaveText("Sending message…");
  await send.getByRole("button", { name: "Fail example send" }).click();
  await expect(send.getByRole("alert")).toContainText(
    "Your message is still here",
  );
  await expect(send.getByRole("textbox")).toContainText(
    "Here is the updated design.",
  );
  await send.getByRole("button", { name: "Send message", exact: true }).click();
  await send.getByRole("button", { name: "Complete example send" }).click();
  await expect(send.getByRole("textbox")).toHaveText("");
});

for (const width of [390, 820, 1440]) {
  test(`playground grows upward and controls fit at ${width}`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.goto(path);
    const playground = page.getByRole("region", {
      name: "Composer playground",
    });
    const input = playground.getByRole("textbox");
    const geometry = () =>
      playground.locator(".message-composer").evaluate((el) => {
        const r = el.getBoundingClientRect();
        return {
          top: r.top + window.scrollY,
          bottom: r.bottom + window.scrollY,
        };
      });
    const before = await geometry();
    await input.fill("One\nTwo\nThree\nFour");
    const after = await geometry();
    expect(after.top).toBeLessThan(before.top);
    expect(after.bottom).toBeCloseTo(before.bottom, 0);
    await playground.getByRole("button", { name: "Toggle formatting" }).click();
    expect(
      await playground.evaluate((el) => el.scrollWidth <= el.clientWidth),
    ).toBe(true);
    await expect(
      playground.getByRole("button", { name: "Send message" }),
    ).toBeVisible();
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= window.innerWidth,
      ),
    ).toBe(true);
  });
}

test("Chat disclosure groups pages and inheritance links live in the description", async ({
  page,
}) => {
  await page.goto(path);
  const nav = page.getByRole("navigation", { name: "Design system" });
  const chat = nav.getByRole("button", { name: "Chat", exact: true });
  await expect(chat).toHaveAttribute("aria-expanded", "true");
  await chat.click();
  await expect(
    nav.getByRole("link", { name: "Composer", exact: true }),
  ).toHaveCount(0);
  await chat.focus();
  await chat.press("Enter");
  await nav.getByRole("link", { name: "Formatting bar", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Formatting bar", exact: true }),
  ).toBeVisible();
  await page.goto(path);
  await expect(
    page.getByText("No Base UI part of its own.", { exact: false }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Implementation details" }),
  ).toHaveCount(0);
  const description = page.locator(".component-page-heading p");
  await expect(description).toContainText("Inherits Button.");
  await description.getByRole("link", { name: "Button", exact: true }).click();
  await expect(page).toHaveURL(/\/design\/components\/button$/);
  await expect(
    page
      .locator(".component-page-heading p")
      .getByRole("link", { name: "Base UI Button" }),
  ).toHaveAttribute("href", "https://base-ui.com/react/components/button");
  await page.goto(path);
  await expect(
    page.getByRole("heading", { name: "02 — Content entered" }),
  ).toHaveCount(0);
});

test("activity remains inspectable on its component page without moving the dock", async ({
  page,
}) => {
  await page.goto("/design/components/activity-rail");
  const dock = page.locator(".conversation-composer-dock");
  const before = await bounds(dock);
  await page.getByRole("switch", { name: "Show agent activity" }).click();
  const after = await bounds(dock);
  expect(after.height).toBeCloseTo(before.height, 0);
  expect(after.y).toBeCloseTo(before.y, 0);
});

test("formats selection and creates a link from the Formatting bar page", async ({
  page,
}) => {
  await page.goto("/design/components/composer-formatting-bar");
  const composer = page.locator(".message-composer");
  const input = composer.getByRole("textbox");
  await input.fill("Format me");
  await input.press("Meta+a");
  await composer.getByRole("button", { name: "Italic" }).click();
  await expect(input.locator("em")).toHaveText("Format me");
  await composer.getByRole("button", { name: "Link" }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();
  await dialog.getByLabel("Link text").fill("Buzz");
  await dialog.getByLabel("Address").fill("https://buzz.example");
  await dialog.getByRole("button", { name: "Add link" }).click();
  await expect(input.locator("a")).toHaveText("Buzz");
  await expect(input.locator("a")).toHaveAttribute(
    "href",
    "https://buzz.example",
  );
});
