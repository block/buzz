import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

const SHOTS = "test-results/team-mentions";
const TEAM_AGENT_PUBKEYS = ["1".repeat(64), "2".repeat(64), "3".repeat(64)];

test.use({ viewport: { width: 1280, height: 720 } });

test("owned team mention stays one chip while targeting its agents", async ({
  page,
}) => {
  await installMockBridge(page, {
    personas: [
      {
        id: "persona-planner",
        displayName: "Planner",
        avatarUrl: "/onboarding/starter-team/fizz.png",
        systemPrompt: "Plan the work.",
      },
      {
        id: "persona-builder",
        displayName: "Builder",
        avatarUrl: "/onboarding/starter-team/honey.png",
        systemPrompt: "Build the solution.",
      },
      {
        id: "persona-reviewer",
        displayName: "Reviewer",
        avatarUrl: "/onboarding/starter-team/bumble.png",
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
    managedAgents: ["Planner", "Builder", "Reviewer"].map((name, index) => ({
      pubkey: TEAM_AGENT_PUBKEYS[index],
      name,
      personaId: `persona-${name.toLowerCase()}`,
      status: "online" as const,
      channelNames: ["general"],
    })),
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
  const teamAvatarStack = teamRow.getByTestId("team-mention-avatar-stack");
  await expect(teamAvatarStack).toBeVisible();
  await expect(
    teamAvatarStack.locator("[data-team-mention-member]"),
  ).toHaveCount(3);
  await expect(teamAvatarStack).toHaveAttribute(
    "aria-label",
    "Launch Team members: Planner, Builder, Reviewer",
  );
  await expect(teamAvatarStack.locator("img")).toHaveCount(3);
  await expect(
    teamRow.getByText("team · 3 agents", { exact: true }).locator("svg"),
  ).toHaveCount(0);

  await waitForAnimations(page);
  await page.screenshot({
    path: `${SHOTS}/01-owned-team-suggestion.png`,
    clip: { x: 240, y: 380, width: 800, height: 320 },
  });

  await input.press("Enter");
  await expect
    .poll(() => input.evaluate((element) => element.textContent))
    .toContain("Coordinate with @Launch Team");
  await expect(input.locator(".agent-mention-highlight")).toHaveCount(1);
  await expect(input.locator(".agent-mention-highlight")).toContainText(
    "Launch Team",
  );
  await expect(input).not.toContainText("Planner");
  await expect(input).not.toContainText("Builder");
  await expect(input).not.toContainText("Reviewer");

  await waitForAnimations(page);
  await page.screenshot({
    path: `${SHOTS}/02-team-mention-chip.png`,
    clip: { x: 240, y: 480, width: 800, height: 220 },
  });

  await page.getByTestId("send-message").click();
  const sentRow = page
    .getByTestId("message-row")
    .filter({ hasText: "Coordinate with" })
    .last();
  const sentChip = sentRow.locator("[data-mention].agent-mention-highlight");
  await expect(sentChip).toHaveCount(1);
  await expect(sentChip).toHaveText("Launch Team");
  await expect(sentRow).not.toContainText("Planner");
  await expect(sentRow).not.toContainText("Builder");
  await expect(sentRow).not.toContainText("Reviewer");

  await expect
    .poll(() =>
      page.evaluate(() => {
        const events = (
          window as Window & {
            __BUZZ_E2E_SIGNED_EVENTS__?: Array<{
              content: string;
              tags: string[][];
            }>;
          }
        ).__BUZZ_E2E_SIGNED_EVENTS__;
        return events?.find(
          (event) => event.content === "Coordinate with @Launch Team",
        )?.tags;
      }),
    )
    .not.toBeUndefined();
  const deliveryTags = await page.evaluate(() => {
    const events = (
      window as Window & {
        __BUZZ_E2E_SIGNED_EVENTS__?: Array<{
          content: string;
          tags: string[][];
        }>;
      }
    ).__BUZZ_E2E_SIGNED_EVENTS__;
    return events?.find(
      (event) => event.content === "Coordinate with @Launch Team",
    )?.tags;
  });
  expect(deliveryTags).toEqual(
    expect.arrayContaining([
      ...TEAM_AGENT_PUBKEYS.map((pubkey) => ["p", pubkey]),
      ["team_mention", "team-launch", "Launch Team"],
    ]),
  );
});
