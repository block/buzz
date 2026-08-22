import { expect, test, type Page } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/persistent-agent-audience";
const CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const AGENT_A = "a".repeat(64);
const AGENT_B = "b".repeat(64);
const THREAD_ROOT_ID = "mock-general-welcome";

async function seedTheme(page: Page, theme: string, accent = "#c0a2f1") {
  await page.addInitScript(
    ({ selectedTheme, selectedAccent }) => {
      window.localStorage.setItem("buzz-theme", selectedTheme);
      window.localStorage.setItem("buzz-accent-color", selectedAccent);
    },
    { selectedTheme: theme, selectedAccent: accent },
  );
}

async function automaticallyMention(
  composer: ReturnType<typeof channelComposer>,
  displayName: string,
) {
  await composer.locator("[data-mention-picker-trigger]").click();
  await composer
    .getByTestId("mention-autocomplete")
    .getByRole("button", { name: `Automatically mention ${displayName}` })
    .click();
  await composer.locator("[data-mention-picker-trigger]").click();
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

async function pressPrimaryShift(page: Page, key: "Enter" | "M") {
  const isMac = await page.evaluate(() =>
    /mac|iphone|ipad|ipod/i.test(navigator.platform),
  );
  await page.keyboard.press(`${isMac ? "Meta" : "Control"}+Shift+${key}`);
}

async function readOutgoingMentionPubkeys(page: Page, content: string) {
  return page.evaluate((expectedContent) => {
    const signedEvent = window.__BUZZ_E2E_SIGNED_EVENTS__?.find(
      (event) => event.content === expectedContent,
    );
    if (signedEvent) {
      return (signedEvent.tags ?? [])
        .filter((tag) => tag[0] === "p" && tag[1])
        .map((tag) => tag[1]);
    }

    for (const entry of window.__BUZZ_E2E_COMMAND_LOG__ ?? []) {
      if (entry.command === "send_channel_message") {
        const payload = entry.payload as
          | { content?: string; mentionPubkeys?: string[] }
          | undefined;
        if (payload?.content === expectedContent) {
          return payload.mentionPubkeys ?? [];
        }
      }

      if (entry.command !== "plugin:websocket|send") continue;
      const data = (
        entry.payload as { message?: { data?: string } } | undefined
      )?.message?.data;
      if (!data) continue;

      try {
        const frame = JSON.parse(data) as [
          string,
          { content?: string; tags?: string[][] },
        ];
        if (frame[0] !== "EVENT" || frame[1]?.content !== expectedContent) {
          continue;
        }
        return (frame[1].tags ?? [])
          .filter((tag) => tag[0] === "p" && tag[1])
          .map((tag) => tag[1]);
      } catch {}
    }

    return null;
  }, content);
}

async function installAudienceFixtures(
  page: Page,
  options: {
    sendMessageDelayMs?: number;
    sendMessageErrors?: string[];
    usersBatchDelayMs?: number;
  } = {},
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

test("automatically mentions multiple agents from the mention picker", async ({
  page,
}) => {
  await installAudienceFixtures(page);
  await openGeneral(page);

  const composer = channelComposer(page);
  await automaticallyMention(composer, "Morgarita");
  await automaticallyMention(composer, "Vogue");

  await expect(
    composer.getByTestId(`composer-address-lock-${AGENT_A}`),
  ).toBeVisible();
  await expect(
    composer.getByTestId(`composer-address-lock-${AGENT_B}`),
  ).toBeVisible();
  await expect(
    composer.getByRole("button", { name: "Manage automatic agent mentions" }),
  ).toBeVisible();
});

test("Tab keeps a manually selected agent as an inline mention", async ({
  page,
}) => {
  await installAudienceFixtures(page);
  await openGeneral(page);

  const composer = channelComposer(page);
  const input = composer.getByTestId("message-input");
  await input.fill("@Mor");
  await expect(composer.getByTestId("mention-autocomplete")).toBeVisible();

  await input.press("Tab");

  await expect(input).toHaveText("@Morgarita ");
  await expect(composer.getByTestId("mention-autocomplete")).toHaveCount(0);
  await expect(
    composer.getByTestId(`composer-address-lock-${AGENT_A}`),
  ).toHaveCount(0);
});

test("primary+Shift+Enter opens the picker, then pins the highlighted agent", async ({
  page,
}) => {
  await installAudienceFixtures(page);
  await openGeneral(page);

  const composer = channelComposer(page);
  const input = composer.getByTestId("message-input");
  await input.fill("draft text");
  await pressPrimaryShift(page, "Enter");

  const menu = composer.getByTestId("mention-autocomplete");
  await expect(menu).toBeVisible();
  await expect(input).toHaveText("draft text");

  await input.fill("@Mor");
  await expect(menu.getByTestId(`mention-suggestion-${AGENT_A}`)).toHaveClass(
    /(?:^|\s)bg-accent(?:\s|$)/,
  );
  await pressPrimaryShift(page, "Enter");

  await expect(menu).toBeVisible();
  await expect(input).toHaveText("");
  await expect(
    composer.getByTestId(`composer-address-lock-${AGENT_A}`),
  ).toBeVisible();
});

test("the mention button opens settings and can undo an address", async ({
  page,
}) => {
  await installAudienceFixtures(page);
  await openGeneral(page);

  const composer = channelComposer(page);
  await automaticallyMention(composer, "Morgarita");
  const input = composer.getByTestId("message-input");
  const ingress = composer.getByRole("button", {
    name: "Manage automatic agent mentions",
  });

  await input.fill("draft text");
  await ingress.click();
  const menu = composer.getByTestId("mention-autocomplete");
  await expect(menu).toBeVisible();
  await expect(input).toHaveText("draft text");
  await page.getByTestId("mention-options-trigger").click();
  await expect(
    page.getByTestId("mention-keep-agents-pinned-toggle"),
  ).toBeVisible();
  const layerBox = await composer
    .getByTestId("mention-autocomplete-layer")
    .boundingBox();
  const optionsBox = await page
    .getByTestId("mention-options-trigger")
    .boundingBox();
  expect(layerBox).not.toBeNull();
  expect(optionsBox).not.toBeNull();
  if (!layerBox || !optionsBox) throw new Error("Mention tray is not laid out");
  await page.mouse.click(layerBox.x + 4, optionsBox.y + optionsBox.height / 2);
  await expect(menu).toHaveCount(0);
  await expect(input).toHaveText("draft text");
  await ingress.click();
  await expect(menu).toBeVisible();
  await expect(page.getByTestId("mention-options-trigger")).toHaveAttribute(
    "aria-expanded",
    "false",
  );
  await expect(
    page.getByTestId("mention-keep-agents-pinned-toggle"),
  ).toHaveCount(0);
  await ingress.click();
  await expect(menu).toHaveCount(0);
  await input.fill("");
  await ingress.click();
  await expect(menu).toBeVisible();
  await expect(page.getByTestId("user-profile-panel")).toHaveCount(0);
  await expect(
    menu.getByRole("button", {
      name: "Stop automatically mentioning Morgarita",
    }),
  ).toHaveAttribute("aria-pressed", "true");

  await menu
    .getByRole("button", { name: "Stop automatically mentioning Morgarita" })
    .click();
  await expect(input).toHaveText("");
  await expect(
    composer.getByRole("button", { name: "Mention someone" }),
  ).toBeVisible();

  await menu
    .getByRole("button", { name: "Mention Morgarita", exact: true })
    .click();
  await expect(input).toHaveText("@Morgarita ");
  await expect(
    composer.getByTestId(`composer-address-lock-${AGENT_A}`),
  ).toHaveCount(0);

  await input.type("later");
  await input.press("Enter");
  await expect(input).toHaveText("");
  await expect
    .poll(() => readOutgoingMentionPubkeys(page, "@Morgarita later"))
    .toContain(AGENT_A);
  await expect(
    composer.getByTestId(`composer-address-lock-${AGENT_A}`),
  ).toHaveCount(1);
});

test("always-mentioned agents remain in the mention button while Enter-send resolves", async ({
  page,
}) => {
  await installAudienceFixtures(page, {
    sendMessageDelayMs: 1_500,
    usersBatchDelayMs: 1_500,
  });
  await openThread(page);

  const composer = threadComposer(page);
  await automaticallyMention(composer, "Morgarita");
  const input = composer.getByTestId("message-input");
  const send = composer.getByTestId("send-message");
  const avatar = composer.getByTestId(`composer-address-lock-${AGENT_A}`);
  const initialPulseVersion = Number(
    await avatar.getAttribute("data-pulse-version"),
  );
  await input.fill("hello");
  await input.press("Enter");

  await expect(input).toHaveText("", { timeout: 500 });
  await expect(avatar).toHaveAttribute(
    "data-pulse-version",
    String(initialPulseVersion + 1),
    { timeout: 500 },
  );
  await expect(
    composer.getByRole("button", { name: "Manage automatic agent mentions" }),
  ).toBeVisible();
  await expect(input).toBeFocused();
  await expect(composer.getByTestId("mention-autocomplete")).toHaveCount(0);

  await expect(send).toBeDisabled();
  await expect
    .poll(() => readOutgoingMentionPubkeys(page, "hello"))
    .toContain(AGENT_A);

  const sentRow = page
    .getByTestId("message-row")
    .filter({ hasText: "hello" })
    .last();
  const addressPrefix = sentRow.locator(
    "[data-mention].agent-mention-highlight",
    { hasText: "Morgarita" },
  );
  await expect(addressPrefix).toBeVisible();
  await expect(addressPrefix).toHaveText("Morgarita");
});

test("a failed always-mentioned send shakes the composer avatar", async ({
  page,
}) => {
  await installAudienceFixtures(page, {
    sendMessageDelayMs: 500,
    sendMessageErrors: ["simulated send failure"],
  });
  await openThread(page);

  const composer = threadComposer(page);
  await automaticallyMention(composer, "Morgarita");
  const input = composer.getByTestId("message-input");
  const avatar = composer.getByTestId(`composer-address-lock-${AGENT_A}`);
  const initialPulseVersion = Number(
    await avatar.getAttribute("data-pulse-version"),
  );
  await expect(avatar).toHaveAttribute("data-shake-version", "0");

  await input.fill("please retry");
  await input.press("Enter");

  await expect(avatar).toHaveAttribute(
    "data-pulse-version",
    String(initialPulseVersion + 1),
    { timeout: 500 },
  );
  await expect(input).toHaveText("please retry");
  await expect(avatar).toHaveAttribute("data-shake-version", "1");
});

test("a manually mentioned agent becomes selected after the message sends", async ({
  page,
}) => {
  await installAudienceFixtures(page, { sendMessageDelayMs: 1_500 });
  await openGeneral(page);

  const composer = channelComposer(page);
  const input = composer.getByTestId("message-input");
  await input.fill("@Mor");
  await expect(composer.getByTestId("mention-autocomplete")).toBeVisible();
  await input.press("Tab");
  await expect(input).toHaveText("@Morgarita ");
  await expect(
    composer.getByTestId(`composer-address-lock-${AGENT_A}`),
  ).toHaveCount(0);

  await input.type("hello");
  await input.press("Enter");

  await expect(input).toHaveText("", { timeout: 500 });
  await expect(input.locator("[data-placeholder]").first()).toHaveAttribute(
    "data-placeholder",
    "Message #general",
    { timeout: 500 },
  );
  await expect(input).toBeFocused();
  await expect(
    composer.getByTestId(`composer-address-lock-${AGENT_A}`),
  ).toBeVisible({ timeout: 2_500 });
  const autoPinToast = page
    .locator("[data-sonner-toast][data-removed='false']")
    .filter({ hasText: "Morgarita will be mentioned automatically" });
  await expect(autoPinToast).not.toContainText(
    "Future messages in this channel will include this agent.",
  );
  const undoAction = autoPinToast.locator("[data-action]");
  await expect(undoAction).toHaveRole("button");
  await expect(undoAction).toHaveText("Undo");
  await expect
    .poll(() => readOutgoingMentionPubkeys(page, "@Morgarita hello"))
    .toContain(AGENT_A);

  await input.fill("follow up");
  await input.press("Enter");
  await expect
    .poll(() => readOutgoingMentionPubkeys(page, "follow up"))
    .toContain(AGENT_A);
});

test("the auto-pin toast can undo and the picker can restore the agent", async ({
  page,
}) => {
  await installAudienceFixtures(page);
  await openGeneral(page);

  const composer = channelComposer(page);
  const input = composer.getByTestId("message-input");
  await input.fill("@Mor");
  await expect(composer.getByTestId("mention-autocomplete")).toBeVisible();
  await input.press("Tab");
  await expect(input).toHaveText("@Morgarita ");
  await input.press("Space");
  await input.type("undo me");
  await input.press("Enter");

  await expect(
    composer.getByTestId(`composer-address-lock-${AGENT_A}`),
  ).toBeVisible();
  const autoPinToast = page
    .locator("[data-sonner-toast][data-removed='false']")
    .filter({ hasText: "Morgarita will be mentioned automatically" });
  await autoPinToast.locator("[data-action]").click();

  await expect(
    composer.getByTestId(`composer-address-lock-${AGENT_A}`),
  ).toHaveCount(0);
  await expect(composer.getByTestId("mention-autocomplete")).toBeVisible();
  await expect(composer.getByTestId("mention-options-trigger")).toHaveAttribute(
    "aria-expanded",
    "true",
  );
  await expect(
    composer.getByTestId("mention-keep-agents-pinned-toggle"),
  ).toBeVisible();

  await input.press("Escape");
  await expect(composer.getByTestId("mention-autocomplete")).toHaveCount(0);
  await composer.locator("[data-mention-picker-trigger]").click();
  await composer
    .getByTestId("mention-autocomplete")
    .getByRole("button", { name: "Mention Morgarita", exact: true })
    .click();
  await expect(input).toHaveText("");
  await expect(
    composer.getByTestId(`composer-address-lock-${AGENT_A}`),
  ).toBeVisible();
});

test("channel automatic mentions carry into threads and stay synchronized", async ({
  page,
}) => {
  await installAudienceFixtures(page);
  await openGeneral(page);
  await automaticallyMention(channelComposer(page), "Morgarita");

  await openThread(page);
  const channelAutomaticMention = channelComposer(page).getByTestId(
    `composer-address-lock-${AGENT_A}`,
  );
  const threadAutomaticMention = threadComposer(page).getByTestId(
    `composer-address-lock-${AGENT_A}`,
  );
  await expect(channelAutomaticMention).toBeVisible();
  await expect(threadAutomaticMention).toBeVisible();

  await threadComposer(page)
    .getByRole("button", { name: "Stop automatically mentioning Morgarita" })
    .click();
  await expect(threadAutomaticMention).toHaveCount(0);
  await expect(channelAutomaticMention).toHaveCount(0);
});

for (const theme of ["buzz", "buzz-dark"]) {
  test(`captures the mention-button placement in ${theme}`, async ({
    page,
  }) => {
    await seedTheme(page, theme);
    await installAudienceFixtures(page);
    await openThread(page);
    const overlay = threadComposer(page);
    const composer = overlay.getByTestId("message-composer");
    await automaticallyMention(overlay, "Morgarita");
    await automaticallyMention(overlay, "Vogue");
    await overlay.getByTestId("message-input").focus();
    await waitForAnimations(page);
    await composer.screenshot({ path: `${SHOTS}/${theme}-mention-button.png` });
  });
}

test("the mention-button placement fits the narrow composer", async ({
  page,
}) => {
  await page.setViewportSize({ width: 700, height: 760 });
  await installAudienceFixtures(page);
  await openThread(page);
  const overlay = threadComposer(page);
  const composer = overlay.getByTestId("message-composer");
  await automaticallyMention(overlay, "Morgarita");
  await automaticallyMention(overlay, "Vogue");
  await expect(overlay.getByTestId("composer-address-locks")).toBeVisible();
  await expect(
    overlay.getByRole("button", { name: "Manage automatic agent mentions" }),
  ).toBeVisible();
  await waitForAnimations(page);
  await composer.screenshot({ path: `${SHOTS}/narrow-mention-button.png` });
});
