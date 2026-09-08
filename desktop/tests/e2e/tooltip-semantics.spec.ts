import { expect, test, type Locator, type Page } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

const THEMES = ["buzz", "catppuccin-mocha"] as const;

async function seedTheme(page: Page, theme: (typeof THEMES)[number]) {
  await page.addInitScript((value) => {
    window.localStorage.setItem("buzz-theme", value);
  }, theme);
}

async function expectPopoverSurface(tooltip: Locator) {
  const colors = await tooltip.evaluate((element) => {
    const sample = document.createElement("div");
    sample.style.cssText =
      "position:fixed;visibility:hidden;background:hsl(var(--popover));color:hsl(var(--popover-foreground))";
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

async function expectMutedSupportingText(
  trigger: Locator,
  expectedText: string | RegExp,
) {
  await trigger.hover();
  const footer = trigger
    .page()
    .getByRole("tooltip")
    .locator('[data-buzz-tooltip-metadata-type=""]');
  await expect(footer).toHaveText(expectedText);
  await expect(footer).toHaveClass(/text-secondary-foreground\/80/);
  await expect(footer).not.toHaveClass(/text-primary-foreground/);
}

for (const theme of THEMES) {
  test(`simple and rich tooltips use the popover surface — ${theme}`, async ({
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
    await expectPopoverSurface(simpleTooltip);

    await page.mouse.move(0, 0);
    await page.getByTestId("channel-intro-action-create-agent").click();
    const dialog = page.getByTestId("add-channel-bot-dialog");
    await expect(dialog).toBeVisible();
    await dialog.getByRole("button", { name: "Contrast Crew" }).hover();

    const richTooltip = page.getByRole("tooltip");
    await expect(richTooltip).toContainText("Checks semantic surface contrast");
    await expect(richTooltip).toContainText("Tooltip Reviewer");
    await expectPopoverSurface(richTooltip);
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

  test(`link tooltip supporting text uses the muted secondary foreground — ${theme}`, async ({
    page,
  }) => {
    await seedTheme(page, theme);
    await installMockBridge(page);
    await page.goto("/");
    await page.getByTestId("channel-general").click();
    const channelId = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
    const owner = "a".repeat(64);
    const issueId = "b".repeat(64);
    const links = [
      `buzz://channel/${channelId}`,
      `buzz://message?channel=${channelId}&id=mock-general-welcome`,
      `buzz://issue?id=${issueId}&owner=${owner}&d=buzz-world`,
    ].join(" ");
    const composer = page.getByTestId("message-input");
    await composer.evaluate((element, text) => {
      const clipboardData = new DataTransfer();
      clipboardData.setData("text/plain", text);
      element.dispatchEvent(
        new ClipboardEvent("paste", {
          bubbles: true,
          cancelable: true,
          clipboardData,
        }),
      );
    }, links);

    const composerChips = composer.locator('[data-composer-buzz-link=""]');
    await expect(composerChips).toHaveCount(3);
    await page.getByTestId("send-message").click();

    const row = page.getByTestId("message-row").last();
    await expect(row).toBeVisible();
    await expectMutedSupportingText(
      row.getByRole("button", { name: "Open channel general" }),
      /Public channel · Active (just now|\d+[mhdw] ago)/,
    );
    await page.mouse.move(0, 0);
    await expectMutedSupportingText(
      row.getByRole("button", { name: "Open message in channel general" }),
      /#general · .+ · (just now|\d+[mhdw] ago)/,
    );
    await page.mouse.move(0, 0);
    await expectMutedSupportingText(
      row.getByRole("button", {
        name: /Open issue .* in repository buzz-world/,
      }),
      "Issue · buzz-world",
    );
  });
}
