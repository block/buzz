import { expect, test, type Page } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/persistent-agent-audience";
const OWNER = "deadbeef".repeat(8);
const CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const AGENT_A = "a".repeat(64);
const AGENT_B = "b".repeat(64);
const SCOPE = `${OWNER}:${CHANNEL_ID}:timeline`;

async function seedAudience(page: Page, pubkeys: string[], theme = "buzz") {
  await page.addInitScript(
    ({ audience, scope, selectedTheme }) => {
      window.localStorage.setItem("buzz:keep-addressed-agents-active", "1");
      window.localStorage.setItem(
        "buzz:persistent-agent-audiences:v2",
        JSON.stringify({ [scope]: audience }),
      );
      window.localStorage.setItem("buzz-theme", selectedTheme);
    },
    { audience: pubkeys, scope: SCOPE, selectedTheme: theme },
  );
}

async function openGeneral(page: Page) {
  await page.goto(`/#/channels/${CHANNEL_ID}`, {
    waitUntil: "domcontentloaded",
  });
  await expect(page.getByTestId("chat-title")).toHaveText("general");
}

async function installAudienceFixtures(page: Page) {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: AGENT_A,
        name: "Morgarita",
        status: "running",
        channelNames: ["general"],
      },
      {
        pubkey: AGENT_B,
        name: "Vogue",
        status: "running",
        channelNames: ["general"],
      },
    ],
  });
}

test("persistent audience states remain visible and deliberate", async ({
  page,
}) => {
  await seedAudience(page, [AGENT_A, AGENT_B]);
  await installAudienceFixtures(page);
  await openGeneral(page);

  const audience = page.getByTestId("persistent-agent-audience");
  await expect(audience).toContainText("Talking to");
  await expect(audience).toContainText("Morgarita");
  await expect(audience).toContainText("Vogue");
  await expect(audience.getByRole("button", { name: "Clear" })).toBeVisible();
  await expect(
    audience.getByRole("button", { name: "Mention someone" }),
  ).toBeVisible();

  await audience.getByRole("button", { name: /Remove Vogue/ }).click();
  await expect(audience).toContainText("Morgarita");
  await expect(audience).not.toContainText("Vogue");

  await audience.getByRole("button", { name: /Remove Morgarita/ }).click();
  await expect(audience).toHaveCount(0);
});

test("audience hides during editing and restores afterward", async ({
  page,
}) => {
  await seedAudience(page, [AGENT_A]);
  await installAudienceFixtures(page);
  await openGeneral(page);

  const input = page.getByTestId("message-input");
  await input.fill("Message to edit");
  await page.getByTestId("send-message").click();
  const row = page.getByTestId("message-row").last();
  await row.scrollIntoViewIfNeeded();
  await row.hover();
  await row.getByLabel("More actions").click({ force: true });
  await page.getByText("Edit message", { exact: true }).click();

  await expect(page.getByTestId("persistent-agent-audience")).toHaveCount(0);
  await expect(page.getByText("Editing message")).toBeVisible();
  await page.getByLabel("Cancel edit").click();
  await expect(page.getByTestId("persistent-agent-audience")).toContainText(
    "Morgarita",
  );
});

for (const theme of ["buzz", "buzz-dark"]) {
  test(`captures multi-agent audience in ${theme}`, async ({ page }) => {
    await seedAudience(page, [AGENT_A, AGENT_B], theme);
    await installAudienceFixtures(page);
    await openGeneral(page);

    const audience = page.getByTestId("persistent-agent-audience");
    await audience.getByRole("button", { name: "Mention someone" }).focus();
    await expect(
      audience.getByRole("button", { name: "Mention someone" }),
    ).toBeFocused();
    await waitForAnimations(page);
    await audience.screenshot({ path: `${SHOTS}/${theme}-multi-focus.png` });
  });
}

test("captures a narrow wrapping audience and single-agent state", async ({
  page,
}) => {
  await page.setViewportSize({ width: 700, height: 760 });
  await seedAudience(page, [AGENT_A, AGENT_B]);
  await installAudienceFixtures(page);
  await openGeneral(page);

  const composer = page.getByTestId("message-composer");
  await waitForAnimations(page);
  await composer.screenshot({ path: `${SHOTS}/narrow-multi-wrap.png` });

  const audience = page.getByTestId("persistent-agent-audience");
  await audience.getByRole("button", { name: /Remove Vogue/ }).click();
  await expect(audience).toContainText("Morgarita");
  await expect(audience.getByRole("button", { name: "Clear" })).toHaveCount(0);
  await waitForAnimations(page);
  await audience.screenshot({ path: `${SHOTS}/narrow-single.png` });
});
