import { expect, test } from "@playwright/test";

test("opens a real channel and keeps protocol metadata out of the transcript", async ({
  page,
}) => {
  await page.goto("/?mock=1");
  await page.getByRole("button", { name: "design", exact: true }).click();
  await expect(page.getByRole("heading", { name: "#design" })).toBeVisible();
  await expect(
    page.getByText(
      "The session should begin as simply as a thought: just type.",
    ),
  ).toBeVisible();
  await expect(page.getByText(/has_more|channel_created/)).toHaveCount(0);
});

test("starts a Session from its channel without naming ceremony", async ({
  page,
}) => {
  await page.goto("/?mock=1");
  await page.getByRole("button", { name: "New Session in design" }).click();
  await expect(page.getByText("From design")).toBeVisible();
  const composer = page.getByRole("textbox", { name: "Start a Session" });
  await composer.fill("Explore a calmer multi-agent conversation");
  await composer.press("Enter");
  await expect(
    page.getByRole("heading", {
      name: "Explore a calmer multi-agent conversation",
    }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", {
      name: "Explore a calmer multi-agent conversation",
    }),
  ).toBeVisible();
});

test("keeps owned agent activity inline and collapses earlier steps", async ({
  page,
}) => {
  await page.goto("/?mock=1");
  await page.getByRole("button", { name: "design", exact: true }).click();
  await expect(
    page.getByRole("region", { name: "Vogue activity" }),
  ).toBeVisible();
  const disclosure = page.getByRole("button", {
    name: /Vogue activity, 5 steps/,
  });
  await disclosure.click();
  await expect(page.getByText("Read the interaction plan")).toBeVisible();
});

test("exposes the focused Agents destination without expanding the app scope", async ({
  page,
}) => {
  await page.goto("/?mock=1");
  await page.getByRole("button", { name: "Agents", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Vogue" })).toBeVisible();
  await expect(
    page.getByText("Shape product interfaces with clarity and taste."),
  ).toBeVisible();
  await page.getByRole("button", { name: "New agent" }).click();
  await expect(page.getByRole("heading", { name: "New agent" })).toBeVisible();
  await page.getByRole("textbox", { name: "Name" }).fill("Scout");
  await page
    .getByRole("textbox", { name: "Instructions" })
    .fill("Help with focused product research.");
  await page.getByRole("button", { name: "Create and start" }).click();
  await expect(page.getByRole("heading", { name: "Scout" })).toBeVisible();
  await expect(
    page.getByText("Help with focused product research."),
  ).toBeVisible();
});
