import { expect, test, type Page } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/persistent-agent-audience";
const OWNER = "deadbeef".repeat(8);
const CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const AGENT_A = "a".repeat(64);
const AGENT_B = "b".repeat(64);
const THREAD_ROOT_ID = "mock-general-welcome";
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
    { audience: pubkeys, scope: CHANNEL_SCOPE, selectedTheme: theme },
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

test("always mentions multiple agents from the mention picker", async ({
  page,
}) => {
  await installAudienceFixtures(page);
  await openGeneral(page);

  const composer = channelComposer(page);
  const input = composer.getByTestId("message-input");
  await input.fill("@Mor");

  const menu = composer.getByTestId("mention-autocomplete");
  await expect(menu).toBeVisible();
  await menu.getByRole("button", { name: "Always mention Morgarita" }).click();
  await expect(menu).toHaveCount(0);
  await expect(input).toHaveText("");
  await expect(input.locator(".agent-mention-highlight")).toHaveCount(0);
  await expect(
    composer.getByRole("button", {
      name: "Stop always mentioning Morgarita",
    }),
  ).toHaveCount(1);

  await input.fill("@Vog");
  await composer
    .getByTestId("mention-autocomplete")
    .getByRole("button", { name: "Always mention Vogue" })
    .click();
  await expect(input).toHaveText("");
  await expect(
    composer.getByRole("button", { name: "Stop always mentioning Vogue" }),
  ).toHaveCount(1);
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

test("the avatar opens the profile and only the hover x removes the agent", async ({
  page,
}) => {
  await seedAudience(page, [AGENT_A]);
  await installAudienceFixtures(page);
  await openGeneral(page);

  const composer = channelComposer(page);
  const avatar = composer.getByRole("button", {
    name: "Open profile for Morgarita",
  });
  const remove = composer.getByRole("button", {
    name: "Stop always mentioning Morgarita",
  });

  await expect(remove).toHaveCSS("opacity", "0");
  await avatar.hover();
  await expect(remove).toHaveCSS("opacity", "1");
  await avatar.click();
  await expect(page.getByTestId("user-profile-panel")).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        ({ scope }) => {
          const stored = JSON.parse(
            localStorage.getItem("buzz:persistent-agent-audiences:v2") ?? "{}",
          );
          return stored[scope] ?? [];
        },
        { scope: CHANNEL_SCOPE },
      ),
    )
    .toEqual([AGENT_A]);

  await avatar.hover();
  await remove.click();
  await expect
    .poll(() =>
      page.evaluate(
        ({ scope }) => {
          const stored = JSON.parse(
            localStorage.getItem("buzz:persistent-agent-audiences:v2") ?? "{}",
          );
          return stored[scope] ?? [];
        },
        { scope: CHANNEL_SCOPE },
      ),
    )
    .toEqual([]);
});

test("reselecting an always-mentioned agent pulses its composer badge", async ({
  page,
}) => {
  await seedAudience(page, [AGENT_A]);
  await installAudienceFixtures(page);
  await openGeneral(page);

  const composer = channelComposer(page);
  const input = composer.getByTestId("message-input");
  const badge = composer.getByTestId(
    `composer-address-lock-mention-badge-${AGENT_A}`,
  );
  await expect(badge).toHaveAttribute("data-pulse-version", "0");

  await input.fill("@Mor");
  await composer
    .getByTestId("mention-autocomplete")
    .getByRole("button", { name: "Mention Morgarita" })
    .click();

  await expect(composer.getByTestId("mention-autocomplete")).toHaveCount(0);
  await expect(input).toHaveText("");
  await expect(input.locator(".agent-mention-highlight")).toHaveCount(0);
  await expect(badge).toHaveAttribute("data-pulse-version", "1");
});

test("always-mentioned agents remain in the tray while Enter-send resolves", async ({
  page,
}) => {
  await seedAudience(page, [AGENT_A]);
  await installAudienceFixtures(page, { sendMessageDelayMs: 1_500 });
  await openThread(page);

  const composer = threadComposer(page);
  const input = composer.getByTestId("message-input");
  const send = composer.getByTestId("send-message");
  const badge = composer.getByTestId(
    `composer-address-lock-mention-badge-${AGENT_A}`,
  );
  await expect(badge).toHaveAttribute("data-pulse-version", "0");
  await input.fill("hello");
  await input.press("Enter");

  await expect(input).toHaveText("", { timeout: 500 });
  await expect(
    composer.getByRole("button", {
      name: "Stop always mentioning Morgarita",
    }),
  ).toBeVisible();
  await expect(input).toBeFocused();
  await expect(composer.getByTestId("mention-autocomplete")).toHaveCount(0);

  await expect(send).toBeDisabled();
  await expect
    .poll(() => readOutgoingMentionPubkeys(page, "hello"))
    .toContain(AGENT_A);
  await expect(badge).toHaveAttribute("data-pulse-version", "1");

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

test("always-mentioned agents restore and can be removed independently", async ({
  page,
}) => {
  await seedAudience(page, [AGENT_B, AGENT_A]);
  await installAudienceFixtures(page);
  await openThread(page);

  const composer = threadComposer(page);
  const input = composer.getByTestId("message-input");
  await expect(input).toHaveText("");
  await expect(composer.getByTestId("composer-address-locks")).toBeVisible();
  await expect(
    composer.getByRole("button", { name: "Stop always mentioning Vogue" }),
  ).toBeVisible();
  await expect(
    composer.getByRole("button", {
      name: "Stop always mentioning Morgarita",
    }),
  ).toBeVisible();

  await composer
    .getByRole("button", { name: "Open profile for Vogue" })
    .hover();
  await composer
    .getByRole("button", { name: "Stop always mentioning Vogue" })
    .click();
  await expect
    .poll(() =>
      page.evaluate(
        ({ scope }) => {
          const stored = JSON.parse(
            localStorage.getItem("buzz:persistent-agent-audiences:v2") ?? "{}",
          );
          return stored[scope] ?? [];
        },
        { scope: CHANNEL_SCOPE },
      ),
    )
    .toEqual([AGENT_A]);

  await input.fill("hello");
  await composer.getByTestId("send-message").click();
  await expect(input).toHaveText("");
  await expect(
    composer.getByRole("button", {
      name: "Stop always mentioning Morgarita",
    }),
  ).toBeVisible();
  await expect
    .poll(() => readOutgoingMentionPubkeys(page, "hello"))
    .toContain(AGENT_A);
  await expect
    .poll(() => readOutgoingMentionPubkeys(page, "hello"))
    .not.toContain(AGENT_B);
});

test("channel always-mentions carry into threads and stay synchronized", async ({
  page,
}) => {
  await seedAudience(page, [AGENT_A]);
  await installAudienceFixtures(page);
  await openGeneral(page);

  await expect(
    channelComposer(page).getByRole("button", {
      name: "Stop always mentioning Morgarita",
    }),
  ).toBeVisible();

  await openThread(page);
  const channelAlwaysMention = channelComposer(page).getByRole("button", {
    name: "Stop always mentioning Morgarita",
  });
  const threadAlwaysMention = threadComposer(page).getByRole("button", {
    name: "Stop always mentioning Morgarita",
  });
  await expect(channelAlwaysMention).toBeVisible();
  await expect(threadAlwaysMention).toBeVisible();

  await threadComposer(page)
    .getByRole("button", { name: "Open profile for Morgarita" })
    .hover();
  await threadAlwaysMention.click();
  await expect(threadAlwaysMention).toHaveCount(0);
  await expect(channelAlwaysMention).toHaveCount(0);
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
    .toEqual([]);
});

for (const theme of ["buzz", "buzz-dark"]) {
  test(`captures the persistent-mention tray in ${theme}`, async ({ page }) => {
    await seedAudience(page, [AGENT_A, AGENT_B], theme);
    await installAudienceFixtures(page);
    await openThread(page);
    const overlay = threadComposer(page);
    const composer = overlay.getByTestId("message-composer");
    await overlay.getByTestId("message-input").focus();
    await waitForAnimations(page);
    await composer.screenshot({
      path: `${SHOTS}/${theme}-persistent-mention-tray.png`,
    });
  });
}

test("the persistent-mention tray fits the narrow composer", async ({
  page,
}) => {
  await page.setViewportSize({ width: 700, height: 760 });
  await seedAudience(page, [AGENT_A, AGENT_B]);
  await installAudienceFixtures(page);
  await openThread(page);
  const overlay = threadComposer(page);
  const composer = overlay.getByTestId("message-composer");
  await expect(overlay.getByTestId("composer-address-locks")).toBeVisible();
  await expect(
    overlay.getByRole("button", {
      name: "Stop always mentioning Morgarita",
    }),
  ).toBeVisible();
  await expect(
    overlay.getByRole("button", { name: "Stop always mentioning Vogue" }),
  ).toBeVisible();
  await waitForAnimations(page);
  await composer.screenshot({
    path: `${SHOTS}/narrow-persistent-mention-tray.png`,
  });
});
