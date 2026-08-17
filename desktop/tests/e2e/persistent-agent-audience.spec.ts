import { expect, test, type Page } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/persistent-agent-audience";
const OWNER = "deadbeef".repeat(8);
const CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const AGENT_A = "a".repeat(64);
const AGENT_B = "b".repeat(64);
const THREAD_ROOT_ID = "mock-general-welcome";
const SCOPE = `${OWNER}:${CHANNEL_ID}:thread:${THREAD_ROOT_ID}`;
const CHANNEL_SCOPE = `${OWNER}:${CHANNEL_ID}:channel`;

async function seedAudience(page: Page, pubkeys: string[], theme = "buzz") {
  await page.addInitScript(
    ({ audience, scope, selectedTheme }) => {
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

async function openThread(page: Page, threadRootId = THREAD_ROOT_ID) {
  await page.goto(
    `/#/channels/${CHANNEL_ID}?messageId=${threadRootId}&thread=${threadRootId}`,
    { waitUntil: "domcontentloaded" },
  );
  await expect(page.getByTestId("message-thread-panel")).toBeVisible();
}

function channelComposer(page: Page) {
  return page.getByTestId("channel-composer-overlay");
}

function threadComposer(page: Page) {
  return page.getByTestId("thread-composer-overlay");
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

test("locks multiple agents from the mention picker without closing it", async ({
  page,
}) => {
  await installAudienceFixtures(page);
  await openGeneral(page);

  const composer = channelComposer(page);
  const input = composer.getByTestId("message-input");
  await composer.getByTestId("message-insert-mention").click();

  const menu = composer.getByTestId("mention-autocomplete");
  await expect(menu).toBeVisible();
  await expect(menu.getByTestId("mention-address-lock-hint")).toHaveText(
    "Hover an agent avatar to keep it addressed",
  );

  const morgaritaRow = menu.getByTestId(`mention-suggestion-${AGENT_A}`);
  const morgaritaLock = menu.getByTestId(`mention-address-lock-${AGENT_A}`);
  await morgaritaRow.hover();
  await expect(morgaritaLock).toBeVisible();
  await morgaritaLock.click();
  await expect(menu).toBeVisible();
  await expect(morgaritaLock).toHaveAttribute("aria-pressed", "true");

  const vogueRow = menu.getByTestId(`mention-suggestion-${AGENT_B}`);
  const vogueLock = menu.getByTestId(`mention-address-lock-${AGENT_B}`);
  await vogueRow.hover();
  await vogueLock.click();

  await expect(menu).toBeVisible();
  await expect(vogueLock).toHaveAttribute("aria-pressed", "true");
  await expect(input).toHaveText("@Morgarita @Vogue ");
  await expect(input.locator(".agent-mention-highlight")).toHaveCount(2);
  await expect
    .poll(() =>
      page.evaluate(
        ({ scope }) => {
          const stored = JSON.parse(
            localStorage.getItem("buzz:persistent-agent-audiences:v2") ?? "{}",
          );
          return stored[scope] ?? null;
        },
        { scope: CHANNEL_SCOPE },
      ),
    )
    .toEqual([AGENT_A, AGENT_B]);
});

test("locked agents transition atomically before Enter-send resolves", async ({
  page,
}) => {
  await seedAudience(page, [AGENT_A]);
  await installAudienceFixtures(page, { sendMessageDelayMs: 1_500 });
  await openThread(page);

  const composer = threadComposer(page);
  const input = composer.getByTestId("message-input");
  const send = composer.getByTestId("send-message");
  await input.fill("@Morgarita hello");
  await input.press("Enter");

  // The network send is still pending, so this is the first observable
  // post-submit editor state rather than the later success hydration pass.
  await expect(input).toHaveText("@Morgarita ", { timeout: 500 });
  await expect(input.locator(".agent-mention-highlight")).toHaveCount(1, {
    timeout: 500,
  });
  await expect(input).toBeFocused();
  await page.waitForTimeout(200);
  await expect(composer.getByTestId("mention-autocomplete")).toHaveCount(0);

  await expect(send).toBeEnabled();
  await expect
    .poll(() =>
      input.evaluate((element) => {
        const selection = window.getSelection();
        const viewDesc = (
          element as HTMLElement & {
            pmViewDesc?: {
              posFromDOM: (node: Node, offset: number, bias: number) => number;
              size: number;
            };
          }
        ).pmViewDesc;
        if (!selection?.anchorNode || !viewDesc) return null;
        const position = viewDesc.posFromDOM(
          selection.anchorNode,
          selection.anchorOffset,
          1,
        );
        // The root view desc includes the document's two boundary tokens,
        // while posFromDOM is relative to the editable root. Converting both
        // to ProseMirror coordinates proves selection.from/to === doc.content.size.
        return {
          empty: selection.isCollapsed,
          text: element.textContent,
          endsWithSpace: element.textContent?.endsWith(" ") ?? false,
          atDocumentEnd: position + 1 === viewDesc.size - 2,
        };
      }),
    )
    .toEqual({
      empty: true,
      text: "@Morgarita ",
      endsWithSpace: true,
      atDocumentEnd: true,
    });

  // Exercise the reported contract, not just its selection prerequisites:
  // immediate typing after restoration must remain outside the mention chip.
  await input.pressSequentially("testing");
  await expect(input).toHaveText("@Morgarita testing");
  await expect(input.locator(".agent-mention-highlight")).toHaveCount(1);
});

test("ordinary agent mentions remain one-shot and return to the placeholder", async ({
  page,
}) => {
  await installAudienceFixtures(page, { sendMessageDelayMs: 1_500 });
  await openGeneral(page);

  const composer = channelComposer(page);
  const input = composer.getByTestId("message-input");
  await input.fill("@Mor");
  await composer
    .getByTestId("mention-autocomplete")
    .getByText("Morgarita", { exact: true })
    .click();
  await input.pressSequentially("hello");
  await expect(input).toHaveText("@Morgarita hello");
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

test("locked agents restore through the native inline mention UI", async ({
  page,
}) => {
  await seedAudience(page, [AGENT_B, AGENT_A]);
  await installAudienceFixtures(page);
  await openThread(page);

  const composer = threadComposer(page);
  const input = composer.getByTestId("message-input");
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

  await composer.getByTestId("send-message").click();
  await expect(input).toContainText("@Morgarita");
  await expect(input).not.toContainText("@Vogue");
  await expect(input.locator(".agent-mention-highlight")).toHaveCount(1);
});

for (const theme of ["buzz", "buzz-dark"]) {
  test(`captures native address-locked mentions in ${theme}`, async ({
    page,
  }) => {
    await seedAudience(page, [AGENT_A, AGENT_B], theme);
    await installAudienceFixtures(page);
    await openThread(page);
    const overlay = threadComposer(page);
    const composer = overlay.getByTestId("message-composer");
    await overlay.getByTestId("message-input").focus();
    await waitForAnimations(page);
    await composer.screenshot({
      path: `${SHOTS}/${theme}-native-mentions.png`,
    });
  });
}

test("native address-locked mentions fit the narrow composer", async ({
  page,
}) => {
  await page.setViewportSize({ width: 700, height: 760 });
  await seedAudience(page, [AGENT_A, AGENT_B]);
  await installAudienceFixtures(page);
  await openThread(page);
  const overlay = threadComposer(page);
  const composer = overlay.getByTestId("message-composer");
  await expect(overlay.getByTestId("message-input")).toContainText(
    "@Morgarita",
  );
  await waitForAnimations(page);
  await composer.screenshot({ path: `${SHOTS}/narrow-native-mentions.png` });
});
