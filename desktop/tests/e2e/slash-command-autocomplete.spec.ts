import { expect, test } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

const OWNER = "deadbeef".repeat(8);
const ALPHA = "a".repeat(64);
const BETA = "b".repeat(64);

test.beforeEach(async ({ page }) => {
  await page.addInitScript(
    ({ owner, alpha, beta }) => {
      localStorage.setItem(
        `buzz-agent-command-catalog.v1:e2e-default-community:${owner}`,
        JSON.stringify({
          version: 1,
          agents: Object.fromEntries(
            [alpha, beta].map((pubkey) => [
              pubkey,
              {
                seq: 1,
                timestamp: "2026-07-23T08:00:00Z",
                commands: [
                  { name: "review", description: "Review the current changes" },
                ],
              },
            ]),
          ),
        }),
      );
    },
    { owner: OWNER, alpha: ALPHA, beta: BETA },
  );
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: ALPHA,
        name: "Alpha",
        channelNames: ["general"],
        status: "running",
      },
      {
        pubkey: BETA,
        name: "Beta",
        channelNames: ["general"],
        status: "running",
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
});

test("persisted commands group by provider and keyboard selection binds the sent recipient", async ({
  page,
}, testInfo) => {
  const input = page.getByTestId("message-input");
  const menu = page.getByTestId("slash-command-autocomplete");
  await input.fill("/rev");
  await expect(
    menu.getByRole("group", { name: "Alpha commands" }),
  ).toBeVisible();
  await expect(
    menu.getByRole("group", { name: "Beta commands" }),
  ).toBeVisible();
  await waitForAnimations(page);
  await menu.screenshot({ path: testInfo.outputPath("slash-commands.png") });
  await input.press("ArrowDown");
  await input.press("Tab");
  await expect(input).toHaveText("@Beta /review ");
  await expect(menu).not.toBeVisible();
  await input.press("Enter");
  await expect
    .poll(() =>
      page.evaluate(() =>
        window.__BUZZ_E2E_SIGNED_EVENTS__
          ?.filter((event) => event.content.includes("/review"))
          .map((event) =>
            event.tags.filter((tag) => tag[0] === "p").map((tag) => tag[1]),
          ),
      ),
    )
    .toContainEqual([BETA]);
});

test("leading mentions filter commands and escape preserves the literal text", async ({
  page,
}) => {
  const input = page.getByTestId("message-input");
  const menu = page.getByTestId("slash-command-autocomplete");
  await input.fill("@Alpha /rev");
  await expect(
    menu.getByRole("group", { name: "Alpha commands" }),
  ).toBeVisible();
  await expect(menu.getByRole("group", { name: "Beta commands" })).toHaveCount(
    0,
  );
  await input.press("Escape");
  await expect(menu).not.toBeVisible();
  await expect(input).toHaveText("@Alpha /rev");
  await input.fill("/review");
  await menu
    .getByRole("group", { name: "Beta commands" })
    .getByRole("button")
    .click();
  await expect(input).toHaveText("@Beta /review ");
});
