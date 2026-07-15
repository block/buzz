import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

const SHOTS = "test-results/team-mentions";

test.use({ viewport: { width: 1280, height: 720 } });

test("owned team mention unfurls into its agents", async ({ page }) => {
  await installMockBridge(page, {
    personas: [
      {
        id: "persona-planner",
        displayName: "Planner",
        systemPrompt: "Plan the work.",
      },
      {
        id: "persona-builder",
        displayName: "Builder",
        systemPrompt: "Build the solution.",
      },
      {
        id: "persona-reviewer",
        displayName: "Reviewer",
        systemPrompt: "Review the result.",
      },
    ],
    teams: [
      {
        id: "team-launch",
        name: "Launch Team",
        description: "Plans, builds, and reviews launches.",
        personaIds: ["persona-planner", "persona-builder", "persona-reviewer"],
      },
    ],
  });

  await page.goto("/");
  await page.getByTestId("channel-general").click();

  const composer = page.getByTestId("message-composer");
  const input = composer.getByTestId("message-input");
  await input.fill("Coordinate with @Launch");

  const autocomplete = composer.getByTestId("mention-autocomplete");
  const teamRow = autocomplete.getByTestId(
    "mention-suggestion-team-team-launch",
  );
  await expect(teamRow).toContainText("Launch Team");
  await expect(teamRow).toContainText("team · 3 agents");

  await waitForAnimations(page);
  await page.screenshot({
    path: `${SHOTS}/01-owned-team-suggestion.png`,
    clip: { x: 240, y: 380, width: 800, height: 320 },
  });

  await input.press("Enter");
  await expect
    .poll(() => input.evaluate((element) => element.textContent))
    .toContain("Coordinate with Launch Team(@Planner @Builder @Reviewer)");
  await expect(input.locator(".agent-mention-highlight")).toHaveCount(3);

  await waitForAnimations(page);
  await page.screenshot({
    path: `${SHOTS}/02-unfurled-team-members.png`,
    clip: { x: 240, y: 480, width: 800, height: 220 },
  });
});
