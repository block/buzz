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

async function installAudienceFixtures(
  page: Page,
  options: { sendMessageDelayMs?: number } = {},
) {
  await installMockBridge(page, {
    ...options,
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

test("persistent agents transition atomically before Enter-send resolves", async ({
  page,
}) => {
  await seedAudience(page, [AGENT_B, AGENT_A]);
  await installAudienceFixtures(page, { sendMessageDelayMs: 1_500 });
  await openGeneral(page);

  const input = page.getByTestId("message-input");
  const send = page.getByTestId("send-message");
  await input.fill("@Morgarita hello");
  await input.press("Enter");

  // The network send is still pending, so this is the first observable
  // post-submit editor state rather than the later success hydration pass.
  await expect(input).toHaveText("@Morgarita ", { timeout: 500 });
  await expect(input.locator(".agent-mention-highlight")).toHaveCount(1, {
    timeout: 500,
  });
  await expect(input).toBeFocused();

  await expect(send).toBeEnabled();
  await expect
    .poll(() =>
      input.evaluate((element) => {
        const selection = window.getSelection();
        return {
          collapsed: selection?.isCollapsed ?? false,
          inside: Boolean(
            selection?.anchorNode && element.contains(selection.anchorNode),
          ),
        };
      }),
    )
    .toEqual({ collapsed: true, inside: true });
});

test("ordinary Enter send transitions directly to the placeholder-ready document", async ({
  page,
}) => {
  await installAudienceFixtures(page, { sendMessageDelayMs: 1_500 });
  await openGeneral(page);

  const input = page.getByTestId("message-input");
  await input.fill("hello");
  await input.press("Enter");

  await expect(input).toHaveText("", { timeout: 500 });
  await expect(input.locator("[data-placeholder]").first()).toHaveAttribute(
    "data-placeholder",
    "Message #general",
    { timeout: 500 },
  );
  await expect(input).toBeFocused();
  await expect
    .poll(() =>
      input.evaluate((element) => {
        const selection = window.getSelection();
        return {
          collapsed: selection?.isCollapsed ?? false,
          inside: Boolean(
            selection?.anchorNode && element.contains(selection.anchorNode),
          ),
        };
      }),
    )
    .toEqual({ collapsed: true, inside: true });
});

test("persistent agents restore through the native inline mention UI", async ({
  page,
}) => {
  await seedAudience(page, [AGENT_B, AGENT_A]);
  await installAudienceFixtures(page);
  await openGeneral(page);

  const input = page.getByTestId("message-input");
  await expect(input).toHaveText("@Vogue @Morgarita ");
  await expect(page.getByText("Talking to", { exact: true })).toHaveCount(0);
  await expect(input.locator(".agent-mention-highlight")).toHaveCount(2);

  await input.fill("@Morgarita hello");
  await expect
    .poll(() =>
      page.evaluate(
        ({ scope }) => {
          const stored = JSON.parse(
            localStorage.getItem("buzz:persistent-agent-audiences:v2") ?? "{}",
          );
          return stored[scope] ?? [];
        },
        { scope: SCOPE },
      ),
    )
    .toEqual([AGENT_A]);

  await page.getByTestId("send-message").click();
  await expect(input).toContainText("@Morgarita");
  await expect(input).not.toContainText("@Vogue");
  await expect(input.locator(".agent-mention-highlight")).toHaveCount(1);
});

for (const theme of ["buzz", "buzz-dark"]) {
  test(`captures native persistent mentions in ${theme}`, async ({ page }) => {
    await seedAudience(page, [AGENT_A, AGENT_B], theme);
    await installAudienceFixtures(page);
    await openGeneral(page);
    const composer = page.getByTestId("message-composer");
    await page.getByTestId("message-input").focus();
    await waitForAnimations(page);
    await composer.screenshot({
      path: `${SHOTS}/${theme}-native-mentions.png`,
    });
  });
}

test("native persistent mentions fit the narrow composer", async ({ page }) => {
  await page.setViewportSize({ width: 700, height: 760 });
  await seedAudience(page, [AGENT_A, AGENT_B]);
  await installAudienceFixtures(page);
  await openGeneral(page);
  const composer = page.getByTestId("message-composer");
  await expect(page.getByTestId("message-input")).toContainText("@Morgarita");
  await waitForAnimations(page);
  await composer.screenshot({ path: `${SHOTS}/narrow-native-mentions.png` });
});
