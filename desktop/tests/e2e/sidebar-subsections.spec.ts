import { expect, test, type Page } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
  await page.goto("/");
});

/** The generated id of the section whose header reads `name`. */
async function sectionId(page: Page, name: string): Promise<string> {
  const title = page
    .locator('[data-testid^="section-title-"]')
    .filter({ hasText: name })
    .first();
  await expect(title).toBeVisible();
  const testId = await title.getAttribute("data-testid");
  return (testId ?? "").replace("section-title-", "");
}

/** The channel list rendered under a section header. */
const sectionBody = (page: Page, id: string) =>
  page.locator(`[id="sidebar-section-${id}"]`);

async function createSectionForChannel(
  page: Page,
  channelTestId: string,
  name: string,
) {
  await page.getByTestId(channelTestId).click({ button: "right" });
  await page.getByRole("menuitem", { name: "Move to section" }).hover();
  await page.getByRole("menuitem", { name: "New section..." }).click();
  await page.getByPlaceholder("Section name").fill(name);
  await page.getByRole("button", { name: "Create" }).click();
}

async function createSubsection(page: Page, parentId: string, name: string) {
  await page.getByTestId(`section-actions-${parentId}`).click();
  await page.getByRole("menuitem", { name: "New subsection..." }).click();
  await expect(
    page.getByRole("heading", { name: "Create subsection" }),
  ).toBeVisible();
  await page.getByPlaceholder("Section name").fill(name);
  await page.getByRole("button", { name: "Create" }).click();
}

/**
 * `target` is the entry's accessible name. A nested section spells its parent
 * out ("Work / Alpha") because its indentation is purely visual.
 */
async function moveChannelToSection(
  page: Page,
  channelTestId: string,
  target: string,
) {
  await page.getByTestId(channelTestId).click({ button: "right" });
  await page.getByRole("menuitem", { name: "Move to section" }).hover();
  await page.getByRole("menuitem", { name: target, exact: true }).click();
}

test("a subsection holds its own channels and survives a reload", async ({
  page,
}) => {
  await createSectionForChannel(page, "channel-random", "Work");
  const workId = await sectionId(page, "Work");
  await expect(
    sectionBody(page, workId).getByTestId("channel-random"),
  ).toBeVisible();

  await createSubsection(page, workId, "Alpha");
  const alphaId = await sectionId(page, "Alpha");
  const alphaGroup = page
    .locator('[data-sidebar-subsection="true"]')
    .filter({ hasText: "Alpha" });
  await expect(alphaGroup).toHaveCount(1);

  await moveChannelToSection(page, "channel-engineering", "Work / Alpha");
  await expect(
    sectionBody(page, alphaId).getByTestId("channel-engineering"),
  ).toBeVisible();

  // Sections live in a relay-synced store, not component state: a reload
  // rebuilds the same tree.
  await page.reload();
  await expect(page.getByTestId(`section-title-${workId}`)).toHaveText("Work");
  await expect(page.getByTestId(`section-title-${alphaId}`)).toHaveText(
    "Alpha",
  );
  await expect(
    page
      .locator('[data-sidebar-subsection="true"]')
      .filter({ hasText: "Alpha" }),
  ).toHaveCount(1);
  await expect(
    sectionBody(page, alphaId).getByTestId("channel-engineering"),
  ).toBeVisible();
  await expect(
    sectionBody(page, workId).getByTestId("channel-random"),
  ).toBeVisible();
});

test("a subsection cannot nest another, and collapsing the parent hides it", async ({
  page,
}) => {
  await createSectionForChannel(page, "channel-random", "Work");
  const workId = await sectionId(page, "Work");
  await createSubsection(page, workId, "Alpha");
  const alphaId = await sectionId(page, "Alpha");

  // The third level is never offered.
  await page.getByTestId(`section-actions-${alphaId}`).click();
  await expect(
    page.getByRole("menuitem", { name: "New subsection..." }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("menuitem", { name: "Move out of subsection" }),
  ).toBeVisible();
  await page.keyboard.press("Escape");

  await page.getByTestId(`section-title-${workId}`).click();
  await expect(page.getByTestId(`section-title-${alphaId}`)).toHaveCount(0);

  await page.getByTestId(`section-title-${workId}`).click();
  await expect(page.getByTestId(`section-title-${alphaId}`)).toBeVisible();
});

test("moving a subsection out returns it to the top level", async ({
  page,
}) => {
  await createSectionForChannel(page, "channel-random", "Work");
  const workId = await sectionId(page, "Work");
  await createSubsection(page, workId, "Alpha");
  const alphaId = await sectionId(page, "Alpha");

  await page.getByTestId(`section-actions-${alphaId}`).click();
  await page.getByRole("menuitem", { name: "Move out of subsection" }).click();

  await expect(page.locator('[data-sidebar-subsection="true"]')).toHaveCount(0);
  await expect(page.getByTestId(`section-title-${alphaId}`)).toBeVisible();
});
