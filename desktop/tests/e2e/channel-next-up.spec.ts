import { expect, test, type Page } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

const OWNER = "deadbeef".repeat(8);
const ALICE =
  "953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f";
const HOME = "cf63feec-21bb-5bf0-a2f8-0e4c3de8ec73";
const RELATED = "9dae0116-799b-5071-a0a8-fdd30a91a35d";
const ADDRESS = `30617:${OWNER}:buzz`;
const MY_TASK_ID = "1".repeat(64);

async function seed(
  page: Page,
  rejectedKinds: number[] = [],
  denseHistory = false,
  oldTask = false,
) {
  await page.addInitScript(
    ({
      owner,
      alice,
      home,
      related,
      address,
      rejectedKinds,
      denseHistory,
      oldTask,
    }) => {
      let number = 0;
      const events: NonNullable<Window["__BUZZ_E2E_EXTRA_PROJECT_EVENTS__"]> =
        [];
      const now = Math.floor(Date.now() / 1000) + 100;
      function task(
        title: string,
        channel: string | null,
        assignees: string[],
        closed = false,
        repo = address,
      ) {
        number += 1;
        const id = number.toString(16).repeat(64);
        events.push({
          id,
          kind: 1621,
          pubkey: owner,
          created_at: now,
          content: "Discuss this task in the original channel.",
          tags: [
            ["a", repo],
            ["subject", title],
            ...(channel ? [["h", channel]] : []),
          ],
        });
        if (assignees.length)
          events.push({
            id: `a${number.toString(16)}`.padEnd(64, "0"),
            kind: 1,
            pubkey: owner,
            created_at: now + 1,
            content: "Assigned this task",
            tags: [
              ["a", repo],
              ["e", id, "", "root"],
              ["t", "assignment"],
              ...assignees.map((key) => ["p", key]),
            ],
          });
        if (closed)
          events.push({
            id: `b${number.toString(16)}`.padEnd(64, "0"),
            kind: 1632,
            pubkey: owner,
            created_at: now + 2,
            content: "",
            tags: [
              ["a", repo],
              ["e", id],
            ],
          });
      }
      task("Prepare launch notes", related, [owner]);
      task("Review the prototype", related, [alice]);
      task("Agree the next milestone", related, [owner, alice]);
      task("Pick a project name", related, []);
      task("Finished experiment", related, [owner], true);
      task("Project home action", home, [owner]);
      task("No recorded origin", null, [owner]);
      task(
        "Unrelated repository action",
        related,
        [owner],
        false,
        `30617:${owner}:not-associated`,
      );
      if (denseHistory) {
        for (let index = 0; index < 1001; index += 1) {
          events.push({
            id: (10000 + index).toString(16).padStart(64, "0"),
            kind: 1621,
            pubkey: owner,
            created_at: now + 500,
            content: "Synthetic dense history",
            tags: [
              ["a", address],
              ["h", home],
            ],
          });
        }
      }
      if (oldTask) {
        for (let index = 0; index < 250; index += 1) {
          events.push({
            id: (20000 + index).toString(16).padStart(64, "0"),
            kind: 1621,
            pubkey: owner,
            created_at: now + 500 + index,
            content: "Recent task in another channel",
            tags: [
              ["a", address],
              ["h", home],
            ],
          });
        }
      }
      window.__BUZZ_E2E_EXTRA_PROJECT_EVENTS__ = events;
      window.__BUZZ_E2E_REJECT_PROJECT_QUERY_KINDS__ = rejectedKinds;
    },
    {
      owner: OWNER,
      alice: ALICE,
      home: HOME,
      related: RELATED,
      address: ADDRESS,
      rejectedKinds,
      denseHistory,
      oldTask,
    },
  );
  await installMockBridge(page);
}

async function openPanel(page: Page, channel = RELATED) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  if (channel === HOME) {
    await page.getByTestId("open-projects-view").click();
    await page.getByTestId("projects-section-projects").click();
    await page
      .locator(
        '[data-testid="project-card-buzz"], [data-testid="project-row-buzz"]',
      )
      .first()
      .click();
  } else {
    await page.getByTestId("channel-random").click();
  }
  await page.getByRole("button", { name: "Show Next up", exact: true }).click();
  const panel = page.getByTestId("channel-next-up-panel");
  await expect(panel).toBeVisible();
  return panel;
}

test("related channel shows its active tasks and filters every assignee", async ({
  page,
}) => {
  await seed(page);
  const panel = await openPanel(page);
  await expect(panel.getByTestId("next-up-task")).toHaveCount(4);
  await expect(panel).not.toContainText("Finished experiment");
  await expect(panel).not.toContainText("Project home action");
  await expect(panel).not.toContainText("No recorded origin");
  await expect(panel).not.toContainText("Unrelated repository action");
  await panel.getByTestId("next-up-person-mine").click();
  await expect(panel.getByTestId("next-up-task")).toHaveCount(2);
  await expect(panel).toContainText("Prepare launch notes");
  await expect(panel).toContainText("Agree the next milestone");
  await panel.getByTestId("next-up-person-teammate").click();
  await panel
    .getByRole("combobox", { name: "Teammate", exact: true })
    .selectOption(ALICE);
  await expect(panel.getByTestId("next-up-task")).toHaveCount(2);
  await expect(panel).toContainText("Review the prototype");
  await expect(panel).not.toContainText("Prepare launch notes");
  await panel.getByTestId("next-up-person-all").click();
  await expect(panel.getByTestId("next-up-task")).toHaveCount(4);
});

test("task opens the existing repository detail and its origin channel", async ({
  page,
}) => {
  await seed(page, [], false, true);
  const panel = await openPanel(page);
  await panel.getByRole("button", { name: /Prepare launch notes/ }).click();
  await expect(page).toHaveURL((url) => {
    const id = new URLSearchParams(url.hash.split("?")[1]).get("issueId");
    return id === MY_TASK_ID || id === JSON.stringify(MY_TASK_ID);
  });
  await expect(
    page.getByTestId("project-issue-comment-composer"),
  ).toBeVisible();
  expect(
    await page.evaluate(
      (id) =>
        window.__BUZZ_E2E_PROJECT_QUERY_FILTERS__?.some(
          (filter) => filter.kinds?.includes(1621) && filter.ids?.includes(id),
        ),
      MY_TASK_ID,
    ),
  ).toBe(true);
  const origin = page.getByRole("button", {
    name: "Open author-claimed origin channel #random",
  });
  await expect(origin).toBeVisible();
  await origin.click();
  await expect(page).toHaveURL(new RegExp(`/channels/${RELATED}`));
});

test("Overview replaces Next up on the project home", async ({ page }) => {
  await page.setViewportSize({ width: 1800, height: 1000 });
  await seed(page);
  const panel = await openPanel(page, HOME);
  await expect(panel).toContainText("Project home action");
  await page
    .getByRole("button", { name: "Show Overview", exact: true })
    .click();
  await expect(panel).not.toBeVisible();
  await expect(
    page.getByRole("button", { name: "Hide Overview", exact: true }),
  ).toBeVisible();
  await page.getByTestId("project-home-context-tasks").click();
  // The existing focus drawer covers this header. Exercise the retained-sheet
  // branch through the real toggle handler without pretending it is clickable.
  await page
    .getByRole("button", { name: "Show Next up", exact: true })
    .dispatchEvent("click");
  await expect(panel).toBeVisible();
  await page
    .getByRole("button", { name: "Show Overview", exact: true })
    .click();
  await expect(panel).not.toBeVisible();
  await expect(
    page.getByRole("button", { name: "Hide Overview", exact: true }),
  ).toBeVisible();
});

test("bound forum channels do not offer an unsupported panel", async ({
  page,
}) => {
  await seed(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("channel-random").click();
  await expect(
    page.getByRole("button", { name: "Show Next up", exact: true }),
  ).toBeVisible();
  await page.evaluate(async (channelId) => {
    window.__BUZZ_E2E_MUTATE_CHANNEL__?.({ channelId, channelType: "forum" });
    await window.__BUZZ_E2E_INVALIDATE_CHANNELS__?.();
  }, RELATED);
  await expect(page.getByTestId("channel-next-up-toggle")).toHaveCount(0);
});

test("project home reuses the panel and switching channels resets person selection", async ({
  page,
}) => {
  await seed(page);
  let panel = await openPanel(page, HOME);
  await expect(panel.getByTestId("next-up-task")).toHaveCount(1);
  await expect(panel).toContainText("Project home action");
  await panel.getByTestId("next-up-person-mine").click();
  await page.getByRole("button", { name: "Close panel", exact: true }).click();
  await expect(panel).not.toBeVisible();
  await page.getByTestId("channel-random").click();
  await page.getByRole("button", { name: "Show Next up", exact: true }).click();
  panel = page.getByTestId("channel-next-up-panel");
  await expect(panel.getByTestId("next-up-task")).toHaveCount(4);
  await expect(panel.getByTestId("next-up-person-all")).toHaveAttribute(
    "aria-pressed",
    "true",
  );
});

test("assignment failures disable person filters and show unknown ownership", async ({
  page,
}) => {
  await seed(page, [1]);
  const panel = await openPanel(page);
  await expect(panel).toContainText("Assignments are unavailable");
  await expect(panel.getByTestId("next-up-person-mine")).toBeDisabled();
  await expect(panel.getByTestId("next-up-task")).toHaveCount(4);
  await expect(
    panel.getByText("Assignment unavailable", { exact: true }),
  ).toHaveCount(4);
  await expect(panel).not.toContainText("Unassigned");
});

test("status failures keep potentially active tasks visible without inventing a status", async ({
  page,
}) => {
  await seed(page, [1632]);
  const panel = await openPanel(page);
  await expect(panel).toContainText("Statuses are unavailable");
  await expect(panel.getByTestId("next-up-task")).toHaveCount(5);
  await expect(
    panel.getByText("Status unavailable", { exact: true }),
  ).toHaveCount(5);
});

test("a failed root read offers refresh instead of claiming there are no tasks", async ({
  page,
}) => {
  await seed(page, [1621]);
  const panel = await openPanel(page);
  await expect(panel).toContainText("Could not refresh channel tasks.", {
    timeout: 20000,
  });
  await expect(panel).not.toContainText("No active tasks");
  await page.evaluate(() => {
    window.__BUZZ_E2E_REJECT_PROJECT_QUERY_KINDS__ = [];
  });
  await page.getByRole("button", { name: "Refresh tasks" }).click();
  await expect(panel.getByTestId("next-up-task")).toHaveCount(4);
});

test("panel works with keyboard and retains a channel draft on close", async ({
  page,
}) => {
  await seed(page);
  const panel = await openPanel(page);
  await panel.getByTestId("next-up-person-mine").focus();
  await page.keyboard.press("Space");
  await expect(panel.getByTestId("next-up-task")).toHaveCount(2);
  await page.getByRole("button", { name: "Close panel", exact: true }).click();
  await expect(panel).not.toBeVisible();
  await waitForAnimations(page);
  await expect(
    page.locator('[aria-label="Channel messages and composer"]'),
  ).not.toHaveAttribute("inert");
  const composer = page.getByTestId("message-input").first();
  await composer.click();
  await composer.pressSequentially("Keep this unsent draft");
  await expect(composer).toContainText("Keep this unsent draft");
  await page.getByRole("button", { name: "Show Next up", exact: true }).click();
  await page.getByRole("button", { name: "Close panel", exact: true }).click();
  await expect(composer).toContainText("Keep this unsent draft");
});

test("dense history shows an incomplete read rather than an empty channel", async ({
  page,
}) => {
  await seed(page, [], true);
  const panel = await openPanel(page);
  await expect(panel).toContainText("Older tasks may be missing.");
  await expect(panel).toContainText("No matching tasks loaded");
  await expect(panel).not.toContainText("No active tasks in this channel");
});

for (const theme of ["light", "dark"]) {
  test(`capture desktop panel in ${theme} and a narrow window`, async ({
    page,
  }, testInfo) => {
    await seed(page);
    await page.addInitScript((theme) => {
      window.localStorage.setItem(
        "buzz-theme",
        theme === "dark" ? "buzz-dark" : "buzz",
      );
    }, theme);
    const panel = await openPanel(page);
    await expect(page.locator("html")).toHaveAttribute(
      "data-buzz-theme",
      theme === "dark" ? "buzz-dark" : "buzz",
    );
    await expect(panel.getByTestId("next-up-task")).toHaveCount(4);
    await waitForAnimations(page);
    await panel.screenshot({
      path: testInfo.outputPath(`next-up-${theme}.png`),
    });
    await page.screenshot({
      path: testInfo.outputPath(`next-up-context-${theme}.png`),
    });
    await page.setViewportSize({ width: 760, height: 800 });
    await waitForAnimations(page);
    await expect(
      panel.getByRole("button", { name: /Prepare launch notes/ }),
    ).toBeVisible();
    await expect
      .poll(() =>
        // Button hit areas extend into the body's padding. Measure its
        // scroll container instead of the unpadded inner section.
        panel.evaluate(
          (element) =>
            element.parentElement !== null &&
            element.parentElement.scrollWidth <=
              element.parentElement.clientWidth,
        ),
      )
      .toBe(true);
    await panel.screenshot({
      path: testInfo.outputPath(`next-up-narrow-${theme}.png`),
    });
    await page.screenshot({
      path: testInfo.outputPath(`next-up-context-narrow-${theme}.png`),
    });
  });
}
