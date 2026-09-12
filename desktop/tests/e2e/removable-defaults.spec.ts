import { expect, test } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";
import { invokeMockCommand } from "../helpers/welcomeTeam";

test("starter templates are an explicit choice and can be removed again", async ({
  page,
}, testInfo) => {
  await installMockBridge(page, { activePersonaIds: [] });
  await page.goto("/");
  await page.getByTestId("open-agents-view").click();
  await expect(
    page.getByRole("button", { name: "Starter templates" }),
  ).toBeVisible();
  await expect(page.getByTestId("persona-agent-row-builtin:fizz")).toHaveCount(
    0,
  );
  await page.screenshot({
    animations: "disabled",
    path: testInfo.outputPath("01-empty.png"),
  });
  await page.getByRole("button", { name: "Starter templates" }).click();
  await page.getByRole("menuitem", { name: "Add Fizz", exact: true }).click();
  await expect(
    page.getByTestId("persona-agent-row-builtin:fizz"),
  ).toBeVisible();
  await page.getByLabel("Open actions for Fizz").click();
  await page.screenshot({
    animations: "disabled",
    path: testInfo.outputPath("02-delete-menu.png"),
  });
  await page.getByRole("menuitem", { name: "Delete", exact: true }).click();
  await expect(page.getByTestId("persona-agent-row-builtin:fizz")).toHaveCount(
    0,
  );
  const saved = await invokeMockCommand<
    Array<{ id: string; is_active: boolean }>
  >(page, "list_personas");
  expect(
    saved.find((persona) => persona.id === "builtin:fizz")?.is_active,
  ).toBe(false);
  await page.reload();
  await page.getByTestId("open-agents-view").click();
  await expect(
    page.getByRole("button", { name: "Starter templates" }),
  ).toBeVisible();
  await expect(page.getByTestId("persona-agent-row-builtin:fizz")).toHaveCount(
    0,
  );
  await expect(
    page.getByRole("button", { name: "Starter templates" }),
  ).toBeVisible();
  await page.screenshot({
    animations: "disabled",
    path: testInfo.outputPath("03-reload-empty.png"),
  });
});

test("Welcome Team can be deleted without deleting a custom lookalike", async ({
  page,
}, testInfo) => {
  await installMockBridge(page, {
    teams: [
      {
        id: "builtin-team:welcome",
        isBuiltin: true,
        name: "Welcome Team",
        personaIds: ["builtin:fizz"],
      },
      { id: "custom:welcome", name: "Welcome Team", personaIds: [] },
    ],
  });
  await page.goto("/");
  await page.getByTestId("open-agents-view").click();
  await expect(
    page.getByRole("button", { name: "Starter templates" }),
  ).toBeVisible();
  const welcome = page.getByTestId("team-card-builtin-team:welcome");
  await welcome
    .getByRole("button", { name: "Welcome Team team actions" })
    .click();
  await page.getByRole("menuitem", { name: "Delete", exact: true }).click();
  await page.screenshot({
    animations: "disabled",
    path: testInfo.outputPath("04-team-delete.png"),
  });
  await page
    .getByRole("alertdialog")
    .getByRole("button", { name: "Delete", exact: true })
    .click();
  await expect(welcome).toHaveCount(0);
  await expect(page.getByTestId("team-card-custom:welcome")).toBeVisible();
  await page.reload();
  await page.getByTestId("open-agents-view").click();
  await expect(
    page.getByRole("button", { name: "Starter templates" }),
  ).toBeVisible();
  await expect(welcome).toHaveCount(0);
  await expect(page.getByTestId("team-card-custom:welcome")).toBeVisible();
  await page.screenshot({
    animations: "disabled",
    path: testInfo.outputPath("05-team-reload.png"),
  });
});
