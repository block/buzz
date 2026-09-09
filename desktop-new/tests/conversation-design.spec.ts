import { expect, test, type Page } from "@playwright/test";
import { waitForAnimations } from "../../desktop/tests/helpers/animations";

const route = "/design/components/conversation";
async function choose(page: Page, name: string) {
  await page.getByRole("combobox", { name: "Preview" }).click();
  await page.getByRole("option", { name, exact: true }).click();
}

test("stable agent identities, independent disclosures, shared replies and notices", async ({
  page,
}) => {
  await page.goto(route);
  const playground = page.getByRole("region", { name: "Session playground" });
  const vogue = playground.getByRole("region", { name: "Vogue activity" });
  const rivet = playground.getByRole("region", { name: "Rivet activity" });
  const ids = () =>
    playground
      .locator("[data-work-id]")
      .evaluateAll((nodes) =>
        nodes.map((node) => node.getAttribute("data-work-id")),
      );
  const originalIds = await ids();
  await expect(vogue.getByText("Only you")).toBeVisible();
  await expect(rivet.getByText("Only you")).toBeVisible();
  const earlier = vogue.getByRole("button", { name: "2 earlier steps" });
  await earlier.focus();
  await page.keyboard.press("Enter");
  await expect(
    vogue.getByText("Read the conversation brief", { exact: true }),
  ).toBeVisible();
  await expect(
    rivet.getByRole("button", { name: "1 earlier step" }),
  ).toHaveAttribute("aria-expanded", "false");
  await rivet
    .getByRole("button", { name: "Checked the activity boundaries" })
    .click();
  await expect(
    rivet.getByText("conversation → shared messages", { exact: false }),
  ).toBeVisible();
  await choose(page, "One agent finished");
  expect(await ids()).toEqual(originalIds);
  await expect(
    vogue.getByRole("button", { name: "5 steps · Reviewed the conversation" }),
  ).toHaveAttribute("aria-expanded", "false");
  await expect(
    rivet.getByRole("button", { name: "Checked the activity boundaries" }),
  ).toHaveAttribute("aria-expanded", "true");
  await expect(
    playground.locator('[data-message-id="vogue-answer"]'),
  ).toBeVisible();
  await expect(
    playground.locator('[data-message-id="rivet-answer"]'),
  ).toHaveCount(0);
  await choose(page, "Both finished");
  await expect(
    playground.locator('[data-message-id="rivet-answer"]'),
  ).toBeVisible();
  await expect(
    vogue.getByRole("button", { name: "Vogue’s notes" }),
  ).toHaveCount(0);
  await choose(page, "Permission needed");
  await expect(rivet.getByText("Allow Rivet to run the checks?")).toBeVisible();
  await rivet.getByRole("button", { name: "Allow once" }).click();
  await expect(rivet.getByText("Allowed in this preview")).toBeVisible();
  await expect(rivet.getByRole("status")).toHaveText("Working");
  await choose(page, "Stopped");
  await rivet
    .getByRole("button", { name: /steps · Checked the handoff/ })
    .click();
  await expect(rivet.locator('[data-status="running"]')).toHaveCount(0);
  await choose(page, "Work failed");
  await expect(rivet.getByText("The checks couldn’t finish")).toBeVisible();
  await rivet.getByRole("button", { name: "Retry in preview" }).click();
  await expect(rivet.getByRole("status")).toHaveText("Working");
  expect(await ids()).toEqual(originalIds);
  await playground.getByRole("textbox").fill("A local-only message");
  await playground
    .getByRole("button", { name: "Send message", exact: true })
    .click();
  await expect(
    playground.getByText("A local-only message", { exact: true }),
  ).toBeVisible();
  await expect(playground.getByText("Sent in this preview only")).toBeVisible();
  await playground.getByRole("textbox").fill("First line\nSecond line");
  await playground
    .getByRole("button", { name: "Send message", exact: true })
    .click();
  const submitted = playground.locator("[data-message-id]").last();
  await expect(submitted.locator(".tiptap p")).toHaveCount(2);
});

test("message actions and recovery examples invoke their supplied callbacks", async ({
  page,
}) => {
  await page.goto("/design/components/conversation-message");
  const reaction = page.getByRole("button", {
    name: "Thumbs up, 1 reactions",
    exact: true,
  });
  await reaction.click();
  await expect(
    page.getByRole("button", { name: "Thumbs up, 2 reactions", exact: true }),
  ).toHaveAttribute("aria-pressed", "true");
  await page.getByRole("button", { name: "Reply to Vogue" }).click();
  await expect(
    page.getByText("Reply target selected: Vogue.", { exact: false }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Retry send" }).click();
  await expect(page.getByText("Sent in this preview")).toBeVisible();
  await page.goto("/design/components/conversation-state");
  await expect(page.getByRole("alert")).toContainText("Couldn’t load");
  await page.getByRole("button", { name: "Retry loading" }).click();
  await expect(page.getByRole("status")).toContainText(
    "A place to work things through",
  );
});

for (const width of [390, 820, 1440]) {
  for (const scheme of ["light", "dark"]) {
    test(`Session holds at ${width} in ${scheme}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 1000 });
      await page.addInitScript(
        (value) => localStorage.setItem("buzz-next-color-scheme", value),
        scheme,
      );
      await page.goto(route);
      await choose(page, "Both finished");
      const playground = page.getByRole("region", {
        name: "Session playground",
      });
      await expect(playground.locator("[data-work-id]")).toHaveCount(2);
      await expect(playground.locator("[data-message-id]")).toHaveCount(3);
      expect(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= window.innerWidth,
        ),
      ).toBe(true);
      expect(
        await playground.evaluate(
          (element) => element.scrollWidth <= element.clientWidth,
        ),
      ).toBe(true);
      await waitForAnimations(page);
      await playground.screenshot({
        path: `test-results/conversation/session-${width}-${scheme}.png`,
      });
    });
  }
}

test("quiet work uses step shimmer and animated Base UI disclosure", async ({
  page,
}) => {
  await page.goto(route);
  const area = page.getByRole("region", { name: "Vogue activity" });
  await expect(
    area.locator(".agent-work-identity .buzz-shimmer-text"),
  ).toHaveCount(0);
  await expect(area.locator(".work-step .buzz-shimmer-text")).toHaveText(
    "Refining how the work settles",
  );
  const trigger = area.getByRole("button", { name: "2 earlier steps" });
  await trigger.hover();
  expect(
    await trigger.evaluate((el) => getComputedStyle(el).backgroundColor),
  ).toBe("rgba(0, 0, 0, 0)");
  await trigger.click();
  const panel = area.locator(".work-history .buzz-accordion-panel").first();
  await expect(panel).toBeVisible();
  expect(
    await panel.evaluate((el) => getComputedStyle(el).transitionProperty),
  ).toBe("height");
  expect(
    await panel.evaluate((el) =>
      getComputedStyle(el).getPropertyValue("--accordion-panel-height"),
    ),
  ).not.toBe("");
  await waitForAnimations(page);
  await page
    .getByRole("region", { name: "Session playground" })
    .screenshot({ path: "test-results/conversation/quiet-expanded.png" });
  await page.emulateMedia({ reducedMotion: "reduce" });
  expect(
    await panel.evaluate((el) => getComputedStyle(el).transitionProperty),
  ).toBe("none");
});
