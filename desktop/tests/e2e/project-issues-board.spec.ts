import { expect, test, type Page } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

test.setTimeout(90_000);

const SHOTS = "test-results/project-issues-board";
const ISSUE_STATUSES = [
  "Triage",
  "Backlog",
  "In Progress",
  "In Review",
  "Done",
  "Closed",
] as const;

async function seedBoard(page: Page, theme: "buzz" | "buzz-dark" = "buzz") {
  await page.addInitScript(
    ({ selectedTheme }) => {
      window.localStorage.setItem(
        "buzz-feature-overrides-v1",
        JSON.stringify({ projects: true }),
      );
      window.localStorage.setItem("buzz.projects.issueViewMode", "board");
      window.localStorage.setItem("buzz-theme", selectedTheme);
    },
    { selectedTheme: theme },
  );
}

async function openBoard(page: Page) {
  await installMockBridge(page);
  const projectsButton = page.getByTestId("open-projects-view");
  for (let attempt = 0; attempt < 3; attempt += 1) {
    await page.goto("/", { waitUntil: "domcontentloaded" });
    try {
      await projectsButton.waitFor({ state: "visible", timeout: 20_000 });
      break;
    } catch (error) {
      if (attempt === 2) throw error;
    }
  }
  await projectsButton.click();
  await page.getByRole("button", { name: "Issues", exact: true }).click();
  await expect(
    page.getByRole("region", { name: "Issues board" }),
  ).toBeVisible();
}

test("issue board persists, scrolls responsively, and opens issue detail", async ({
  page,
}) => {
  await page.setViewportSize({ height: 900, width: 1440 });
  await seedBoard(page);
  await openBoard(page);

  const board = page.getByRole("region", { name: "Issues board" });
  for (const status of ISSUE_STATUSES) {
    await expect(
      board.getByRole("heading", { name: status, exact: true }),
    ).toBeVisible();
  }
  await expect(
    board.locator('[data-testid^="projects-issue-board-card-"]').first(),
  ).toBeVisible();

  await page.getByRole("button", { name: "List layout" }).click();
  await expect(
    page.locator('[data-testid^="projects-issue-row-"]').first(),
  ).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(() =>
        window.localStorage.getItem("buzz.projects.issueViewMode"),
      ),
    )
    .toBe("list");

  await page.getByRole("button", { name: "Board layout" }).click();
  await expect(board).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(() =>
        window.localStorage.getItem("buzz.projects.issueViewMode"),
      ),
    )
    .toBe("board");

  await waitForAnimations(page);
  await page.screenshot({
    path: `${SHOTS}/01-board-light.png`,
  });

  await page.setViewportSize({ height: 760, width: 820 });
  await expect
    .poll(() =>
      board.evaluate((element) => element.scrollWidth > element.clientWidth),
    )
    .toBe(true);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBe(true);
  await waitForAnimations(page);
  await page.screenshot({
    path: `${SHOTS}/02-board-narrow.png`,
  });

  const firstCard = board
    .locator('[data-testid^="projects-issue-board-card-"]')
    .first();
  const cardName = await firstCard.getAttribute("aria-label");
  await firstCard.focus();
  await expect(firstCard).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(
    page.getByRole("heading", {
      name: cardName?.replace(/^Open /, "").replace(/ in .*$/, ""),
    }),
  ).toBeVisible();
});

test("issue board renders in the Buzz dark theme", async ({ page }) => {
  await page.setViewportSize({ height: 900, width: 1440 });
  await seedBoard(page, "buzz-dark");
  await openBoard(page);

  await expect(page.locator("html")).toHaveClass(/dark/);
  await waitForAnimations(page);
  await page.screenshot({
    path: `${SHOTS}/03-board-dark.png`,
  });
});
