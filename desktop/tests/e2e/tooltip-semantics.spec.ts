import { expect, test, type Locator, type Page } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

const THEMES = ["buzz", "catppuccin-mocha"] as const;

async function seedTheme(page: Page, theme: (typeof THEMES)[number]) {
  await page.addInitScript((value) => {
    window.localStorage.setItem("buzz-theme", value);
  }, theme);
}

async function expectSecondarySurface(tooltip: Locator) {
  const colors = await tooltip.evaluate((element) => {
    const sample = document.createElement("div");
    sample.style.cssText =
      "position:fixed;visibility:hidden;background:hsl(var(--secondary));color:hsl(var(--secondary-foreground))";
    document.body.append(sample);
    const tooltipStyle = getComputedStyle(element);
    const sampleStyle = getComputedStyle(sample);
    const result = {
      background: tooltipStyle.backgroundColor,
      color: tooltipStyle.color,
      expectedBackground: sampleStyle.backgroundColor,
      expectedColor: sampleStyle.color,
    };
    sample.remove();
    return result;
  });

  expect(colors.background).toBe(colors.expectedBackground);
  expect(colors.color).toBe(colors.expectedColor);
}

for (const theme of THEMES) {
  test(`simple and rich tooltips use the secondary surface — ${theme}`, async ({
    page,
  }) => {
    await seedTheme(page, theme);
    await installMockBridge(page, {
      personas: [
        {
          id: "tooltip-reviewer",
          displayName: "Tooltip Reviewer",
          systemPrompt: "Review tooltip contrast.",
        },
      ],
      teams: [
        {
          id: "tooltip-team",
          name: "Contrast Crew",
          description: "Checks semantic surface contrast",
          personaIds: ["tooltip-reviewer"],
        },
      ],
    });
    await page.goto("/");

    await page.getByTestId("channel-random").click();
    await page.getByTestId("channel-members-trigger").hover();
    const simpleTooltip = page.getByRole("tooltip");
    await expect(simpleTooltip).toHaveText("Channel members");
    await expectSecondarySurface(simpleTooltip);

    await page.mouse.move(0, 0);
    await page.getByTestId("channel-intro-action-create-agent").click();
    const dialog = page.getByTestId("add-channel-bot-dialog");
    await expect(dialog).toBeVisible();
    await dialog.getByRole("button", { name: "Contrast Crew" }).hover();

    const richTooltip = page.getByRole("tooltip");
    await expect(richTooltip).toContainText("Checks semantic surface contrast");
    await expect(richTooltip).toContainText("Tooltip Reviewer");
    await expectSecondarySurface(richTooltip);
    await expect(
      richTooltip.getByText("Checks semantic surface contrast"),
    ).toHaveClass(/text-secondary-foreground\/80/);
    await expect(richTooltip.getByText("Tooltip Reviewer")).toHaveClass(
      /text-secondary-foreground/,
    );
    await expect(
      richTooltip.getByTestId("team-tooltip-persona-chip"),
    ).toHaveClass(/bg-secondary-foreground\/10/);
    await expect(
      richTooltip.getByTestId("team-tooltip-persona-avatar"),
    ).toHaveClass(/bg-secondary-foreground\/20.*text-secondary-foreground/);

    await waitForAnimations(page);
    await dialog.screenshot({
      path: `test-results/tooltip-semantics/${theme}-team-tooltip.png`,
    });
  });
}
