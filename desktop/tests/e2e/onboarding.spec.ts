import { hexToBytes } from "@noble/hashes/utils.js";
import { expect, test, type Locator, type Page } from "@playwright/test";
import { nsecEncode } from "nostr-tools/nip19";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

type RelayConnectionState =
  | "connected"
  | "connecting"
  | "disconnected"
  | "idle"
  | "reconnecting"
  | "stalled";

/**
 * Drive the mock relay connection state via the E2E bridge seam.
 * When targeting a degraded state, waits for baseline "connected" first so
 * the mock auth handshake can't race the override back to "connected".
 */
async function setRelayConnectionState(
  page: Page,
  state: RelayConnectionState,
) {
  if (state !== "connected") {
    await page.waitForFunction(() => {
      const win = window as Window & {
        __BUZZ_E2E_GET_RELAY_CONNECTION_STATE__?: () => string;
        __BUZZ_E2E_SET_RELAY_CONNECTION_STATE__?: unknown;
      };
      return (
        typeof win.__BUZZ_E2E_SET_RELAY_CONNECTION_STATE__ === "function" &&
        typeof win.__BUZZ_E2E_GET_RELAY_CONNECTION_STATE__ === "function" &&
        win.__BUZZ_E2E_GET_RELAY_CONNECTION_STATE__() === "connected"
      );
    });
  } else {
    await page.waitForFunction(
      () =>
        typeof (
          window as Window & {
            __BUZZ_E2E_SET_RELAY_CONNECTION_STATE__?: unknown;
          }
        ).__BUZZ_E2E_SET_RELAY_CONNECTION_STATE__ === "function",
    );
  }
  await page.evaluate((nextState) => {
    const testWindow = window as Window & {
      __BUZZ_E2E_SET_RELAY_CONNECTION_STATE__?: (
        state: RelayConnectionState,
      ) => void;
    };
    const setConnectionState =
      testWindow.__BUZZ_E2E_SET_RELAY_CONNECTION_STATE__;
    if (!setConnectionState) {
      throw new Error("Mock relay connection state helper is not installed.");
    }
    setConnectionState(nextState);
  }, state);
}

const E2E_IDENTITY_OVERRIDE_STORAGE_KEY = "buzz:e2e-identity-override.v1";
const HOME_SEEN_STORAGE_KEY_PREFIX = "buzz-home-feed-seen.v1:";
const DEFAULT_MOCK_PUBKEY = "deadbeef".repeat(8);
const BLANK_TYLER_IDENTITY = {
  ...TEST_IDENTITIES.tyler,
  username: "",
};
const BLANK_AVATAR_PLACEHOLDER_IDENTITY = {
  ...TEST_IDENTITIES.tyler,
  pubkey: "1".repeat(64),
  username: "",
};
const BLANK_AVATAR_EMOJI_IDENTITY = {
  ...TEST_IDENTITIES.tyler,
  pubkey: "2".repeat(64),
  username: "",
};
const FIRST_RUN_ALICE = {
  ...TEST_IDENTITIES.alice,
  username: "",
};

type TestIdentity = {
  privateKey: string;
  pubkey: string;
  username: string;
};

async function seedActiveIdentity(page: Page, identity: TestIdentity) {
  await page.addInitScript(
    ({ identity: nextIdentity, storageKey }) => {
      window.localStorage.setItem(storageKey, JSON.stringify(nextIdentity));
    },
    {
      identity,
      storageKey: E2E_IDENTITY_OVERRIDE_STORAGE_KEY,
    },
  );
}

async function seedOnboardingCompletion(page: Page, pubkey: string) {
  await page.addInitScript(
    ({ storageKey }) => {
      window.localStorage.setItem(storageKey, "true");
    },
    {
      storageKey: `buzz-onboarding-complete.v1:${pubkey}`,
    },
  );
}

async function readHomeSeenStorageKeys(page: Page) {
  return page.evaluate((prefix) => {
    return Object.keys(window.localStorage).filter((key) =>
      key.startsWith(prefix),
    );
  }, HOME_SEEN_STORAGE_KEY_PREFIX);
}

async function expectNoHomeSeenEntries(page: Page) {
  await expect.poll(async () => readHomeSeenStorageKeys(page)).toEqual([]);
}

async function selectFirstEmojiFromPicker(page: Page) {
  const picker = page.locator("em-emoji-picker");
  await expect(picker).toBeVisible();
  await expect
    .poll(() =>
      picker.evaluate((element) =>
        Boolean(element.shadowRoot?.querySelector(".scroll button")),
      ),
    )
    .toBe(true);
  await picker.evaluate((element) => {
    const button = element.shadowRoot?.querySelector(".scroll button");
    if (!(button instanceof HTMLElement)) {
      throw new Error("Emoji picker did not render an emoji button.");
    }
    button.click();
  });
}

async function expectShellHidden(page: Page) {
  await expect(page.getByTestId("app-sidebar")).toHaveCount(0);
  await expect(page.getByTestId("chat-title")).toHaveCount(0);
}

async function expectHomeView(page: Page) {
  await expect(page.getByTestId("home-inbox-list")).toBeVisible();
}

async function expectWiderThanTall(locator: Locator) {
  const box = await locator.boundingBox();
  if (!box) {
    throw new Error("Could not measure welcome intro action card");
  }

  expect(box.width).toBeGreaterThan(box.height);
}

async function expectIntroActionIconStackedAboveTitle(
  action: Locator,
  testId: string,
) {
  const iconBox = await action.getByTestId(`${testId}-icon`).boundingBox();
  const titleBox = await action.getByTestId(`${testId}-title`).boundingBox();
  if (!iconBox || !titleBox) {
    throw new Error("Could not measure welcome intro action content");
  }

  expect(titleBox.y).toBeGreaterThan(iconBox.y + iconBox.height);
}

async function expectWelcomeComposerBannerLayout(page: Page) {
  const banner = page.getByTestId("welcome-composer-guide-banner");
  const personaMention = page.getByTestId("welcome-composer-persona-mention");
  const composer = page.getByTestId("message-composer");
  const bannerBox = await banner.boundingBox();
  const personaMentionBox = await personaMention.boundingBox();
  const composerBox = await composer.boundingBox();

  if (!bannerBox || !personaMentionBox || !composerBox) {
    throw new Error("Could not measure welcome composer banner layout");
  }

  expect(
    await composer.getByTestId("welcome-composer-guide-banner").count(),
  ).toBe(0);
  expect(bannerBox.y).toBeLessThan(composerBox.y);
  expect(bannerBox.y + bannerBox.height).toBeGreaterThan(composerBox.y);

  const radii = await banner.evaluate((element) => {
    const styles = window.getComputedStyle(element);
    return {
      bottomLeft: styles.borderBottomLeftRadius,
      bottomRight: styles.borderBottomRightRadius,
      topLeft: styles.borderTopLeftRadius,
      topRight: styles.borderTopRightRadius,
    };
  });

  expect(radii.topLeft).toBe(radii.topRight);
  expect(radii.bottomLeft).toBe("0px");
  expect(radii.bottomRight).toBe("0px");
  expect(personaMentionBox.width).toBeGreaterThan(0);
}

async function expectWelcomePersonaMention(page: Page) {
  const banner = page.getByTestId("welcome-composer-guide-banner");
  const personaMention = page.getByTestId("welcome-composer-persona-mention");
  await expect(personaMention).toBeVisible();
  await expect(personaMention).toHaveAttribute("data-persona-options", "Fizz");
  await expect(personaMention).toHaveAttribute("data-active-persona", "Fizz");
  await expect(personaMention).toHaveAttribute(
    "data-animation-target",
    "per-character",
  );

  const activePersona = await personaMention.getAttribute(
    "data-active-persona",
  );
  expect(activePersona).not.toBeNull();
  await expect(personaMention).toContainText(`@${activePersona}`);
  expect(
    await personaMention
      .getByTestId("welcome-composer-persona-character")
      .count(),
  ).toBeGreaterThanOrEqual(4);

  const transition = await personaMention.evaluate((element) => {
    const styles = window.getComputedStyle(element);
    const durationMs = Number(
      element.getAttribute("data-width-animation-duration-ms"),
    );
    return {
      duration: styles.transitionDuration,
      durationMs,
      property: styles.transitionProperty,
    };
  });
  expect(transition.durationMs).toBeGreaterThanOrEqual(700);
  expect(transition.durationMs).toBeLessThanOrEqual(740);
  expect(Math.round(Number.parseFloat(transition.duration) * 1000)).toBe(
    transition.durationMs,
  );
  expect(transition.property).toContain("width");

  const alignment = await personaMention.evaluate((element) => {
    const mentionStyles = window.getComputedStyle(element);
    const bannerStyles = window.getComputedStyle(
      element.closest('[data-testid="welcome-composer-guide-banner"]') ??
        element,
    );
    return {
      display: mentionStyles.display,
      lineHeightMatchesBanner:
        mentionStyles.lineHeight === bannerStyles.lineHeight,
      verticalAlign: mentionStyles.verticalAlign,
    };
  });
  expect(alignment.display).toBe("inline-block");
  expect(alignment.verticalAlign).toBe("baseline");
  expect(alignment.lineHeightMatchesBanner).toBe(true);
  await expect(banner).toContainText("Try mentioning");
}

async function expectWelcomeView(page: Page) {
  await expect(page).toHaveURL(/#\/channels\/[^/?#]+$/);
  await expect(page.getByTestId("channel-Welcome")).toBeVisible();
  await expect(page.getByTestId("chat-title")).toContainText("Welcome");
  await expect(page.getByTestId("channel-ephemeral-Welcome")).toHaveCount(0);
  await expect(page.getByTestId("chat-ephemeral-badge")).toHaveCount(0);
  await expect(page.getByTestId("message-unread-pill")).toHaveCount(0);
  await expect(page.getByTestId("message-channel-intro")).toBeVisible();
  await expect(page.getByTestId("message-channel-intro")).toContainText(
    "This is the beginning of the private welcome channel.",
  );
  await expect(page.getByTestId("message-channel-intro")).not.toContainText(
    "A few good first steps",
  );
  await expect(
    page.getByTestId("welcome-intro-action-create-channel"),
  ).toBeVisible();
  await expectWiderThanTall(
    page.getByTestId("welcome-intro-action-create-channel"),
  );
  await expectIntroActionIconStackedAboveTitle(
    page.getByTestId("welcome-intro-action-create-channel"),
    "welcome-intro-action-create-channel",
  );
  await expect(
    page.getByTestId("welcome-intro-action-create-channel-title"),
  ).toHaveText("Create a channel");
  await expect(
    page.getByTestId("welcome-intro-action-create-channel-title"),
  ).toHaveCSS("white-space", "normal");
  await expect(
    page.getByTestId("welcome-intro-action-create-channel-description"),
  ).toHaveCount(0);
  await expect(
    page.getByTestId("welcome-intro-action-create-agent"),
  ).toBeVisible();
  await expectWiderThanTall(
    page.getByTestId("welcome-intro-action-create-agent"),
  );
  await expectIntroActionIconStackedAboveTitle(
    page.getByTestId("welcome-intro-action-create-agent"),
    "welcome-intro-action-create-agent",
  );
  await expect(
    page.getByTestId("welcome-intro-action-create-agent-title"),
  ).toHaveText("Create a custom agent");
  await expect(
    page.getByTestId("welcome-intro-action-create-agent-description"),
  ).toHaveCount(0);
  await expect(page.getByTestId("message-composer")).toBeVisible();
  await expect(page.getByTestId("welcome-composer-guide-banner")).toBeVisible();
  await expect(page.getByTestId("welcome-composer-guide-banner")).toContainText(
    "Try mentioning",
  );
  await expect(page.getByTestId("welcome-composer-guide-banner")).toContainText(
    "to chat with an agent in this channel.",
  );
  await expectWelcomePersonaMention(page);
  await expectWelcomeComposerBannerLayout(page);
}

async function expectWelcomeComposerBannerCompletesAfterPersonaMention(
  page: Page,
) {
  const banner = page.getByTestId("welcome-composer-guide-banner");
  const channelIntro = page.getByTestId("message-channel-intro");

  await page.getByTestId("message-input").fill("Thanks @Fizz");
  await page.getByTestId("send-message").click();

  await expect(banner).toHaveAttribute("data-state", "complete");
  await expect(banner).toHaveAttribute("data-tone", "success");
  await expect(
    banner.getByTestId("welcome-composer-complete-icon"),
  ).toBeVisible();
  await expect(
    banner.locator('[data-animation-target="success-icon"]'),
  ).toBeVisible();
  await expect(
    banner.locator('[data-animation-target="success-copy"]'),
  ).toBeVisible();
  await expect(banner).toContainText("Nice work.");
  await expect(banner).not.toContainText("Try mentioning");
  await expect(channelIntro).toBeVisible();
}

async function getMockChannels(page: Page) {
  return page.evaluate(async () => {
    const bridgeWindow = window as Window & {
      __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
        command: string,
        payload?: Record<string, unknown>,
      ) => Promise<unknown>;
      __TAURI_INTERNALS__?: {
        invoke?: (
          command: string,
          payload?: Record<string, unknown>,
        ) => Promise<unknown>;
      };
    };
    const invoke =
      bridgeWindow.__BUZZ_E2E_INVOKE_MOCK_COMMAND__ ??
      bridgeWindow.__TAURI_INTERNALS__?.invoke;

    if (!invoke) {
      throw new Error("Mock invoke bridge is unavailable.");
    }

    return (await invoke("get_channels")) as Array<{
      id: string;
      name: string;
      channel_type: string;
      visibility: "open" | "private";
      member_count: number;
      is_member: boolean;
      ttl_seconds: number | null;
    }>;
  });
}

async function invokeMockCommand<T>(
  page: Page,
  command: string,
  payload?: Record<string, unknown>,
) {
  return page.evaluate(
    async ({ command, payload }) => {
      const bridgeWindow = window as Window & {
        __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
          command: string,
          payload?: Record<string, unknown>,
        ) => Promise<unknown>;
        __TAURI_INTERNALS__?: {
          invoke?: (
            command: string,
            payload?: Record<string, unknown>,
          ) => Promise<unknown>;
        };
      };
      const invoke =
        bridgeWindow.__BUZZ_E2E_INVOKE_MOCK_COMMAND__ ??
        bridgeWindow.__TAURI_INTERNALS__?.invoke;

      if (!invoke) {
        throw new Error("Mock invoke bridge is unavailable.");
      }

      return (await invoke(command, payload)) as T;
    },
    { command, payload },
  );
}

async function getWelcomeChannelId(page: Page) {
  const channels = await getMockChannels(page);
  return (
    channels.find(
      (channel) =>
        channel.name === "Welcome" && channel.visibility === "private",
    )?.id ?? null
  );
}

async function expectPrivateWelcomeChannel(page: Page) {
  await expect
    .poll(async () => {
      const channels = await getMockChannels(page);
      const welcomeChannel = channels.find(
        (channel) =>
          channel.name === "Welcome" && channel.visibility === "private",
      );

      if (!welcomeChannel) {
        return null;
      }

      return {
        channelType: welcomeChannel.channel_type,
        isMember: welcomeChannel.is_member,
        memberCount: welcomeChannel.member_count,
        ttlSeconds: welcomeChannel.ttl_seconds,
        visibility: welcomeChannel.visibility,
      };
    })
    .toEqual({
      channelType: "stream",
      isMember: true,
      memberCount: 2,
      ttlSeconds: null,
      visibility: "private",
    });
}

async function expectWelcomeGuideIntro(
  page: Page,
  { expectVisible = true }: { expectVisible?: boolean } = {},
) {
  await expect
    .poll(async () => {
      const channelId = await getWelcomeChannelId(page);
      if (!channelId) {
        return null;
      }

      const [members, agents, introSearch] = await Promise.all([
        invokeMockCommand<{
          members: Array<{ pubkey: string; role: string; is_agent: boolean }>;
        }>(page, "get_channel_members", { channelId }),
        invokeMockCommand<
          Array<{ pubkey: string; name: string; persona_id: string | null }>
        >(page, "list_managed_agents"),
        invokeMockCommand<{
          hits: Array<{ pubkey: string; content: string }>;
        }>(page, "search_messages", {
          q: "Hi, I'm Fizz",
          limit: 10,
        }),
      ]);
      const fizz = agents.find(
        (agent) => agent.name === "Fizz" && agent.persona_id === "builtin:fizz",
      );
      const fizzMember = fizz
        ? members.members.find((member) => member.pubkey === fizz.pubkey)
        : null;
      const intro = fizz
        ? introSearch.hits.find((hit) => hit.pubkey === fizz.pubkey)
        : null;
      const fizzWelcomeHits = fizz
        ? introSearch.hits.filter((hit) => hit.pubkey === fizz.pubkey)
        : [];
      const profileAvatarUrl = fizz
        ? (
            await invokeMockCommand<{
              profiles: Record<string, { avatar_url: string | null }>;
            }>(page, "get_users_batch", {
              pubkeys: [fizz.pubkey],
            })
          ).profiles[fizz.pubkey]?.avatar_url
        : null;

      return {
        fizzIsBot: fizzMember?.role === "bot" && fizzMember.is_agent,
        fizzPersonaId: fizz?.persona_id ?? null,
        introContent: intro?.content ?? null,
        introMatchesFizz: Boolean(fizz && intro?.pubkey === fizz.pubkey),
        fizzWelcomeHitCount: fizzWelcomeHits.length,
        profileAvatarUrl,
      };
    })
    .toEqual({
      fizzIsBot: true,
      fizzPersonaId: "builtin:fizz",
      introContent:
        "Hi, I'm Fizz. Welcome to Buzz.\n\nI can help you get oriented, answer questions, and make the first few steps feel less mysterious.\n\nFeel free to ask me what else you can do in Buzz, or just talk through what you want to build.",
      introMatchesFizz: true,
      fizzWelcomeHitCount: 1,
      profileAvatarUrl: null,
    });

  if (expectVisible) {
    await expect(page.getByTestId("message-timeline")).toContainText(
      "Hi, I'm Fizz. Welcome to Buzz.",
    );
    await expect(page.getByTestId("message-timeline")).toContainText(
      "Feel free to ask me what else you can do in Buzz",
    );
    await expect(
      page.getByTestId("message-timeline-day-divider"),
    ).toBeVisible();
    await expect(page.getByTestId("system-message-row")).toHaveCount(0);
  }
}

async function expectIncompleteOnboarding(page: Page) {
  await expect(page.getByTestId("onboarding-gate")).toBeVisible();
  await expectShellHidden(page);
  await expect(page.getByTestId("onboarding-page-1")).toBeVisible();
  await expect(page.getByTestId("onboarding-display-name")).toHaveValue("");
}

async function continueToSetupPage(page: Page) {
  await page.getByTestId("onboarding-next").click();
  await expect(page.getByTestId("onboarding-page-avatar")).toBeVisible();
  await page
    .getByTestId("onboarding-avatar-url")
    .fill("https://example.com/onboarding-avatar.png");
  await page.getByTestId("onboarding-next").click();
  await expect(page.getByTestId("onboarding-page-theme")).toBeVisible();
  await page.getByTestId("onboarding-theme-option-github-light").click();
  await expect
    .poll(() => page.evaluate(() => window.localStorage.getItem("buzz-theme")))
    .toBe("github-light");
  await page.getByTestId("onboarding-next").click();
  await expect(page.getByTestId("onboarding-page-2")).toBeVisible();
}

test("completed users skip the loading gate while profile is still settling", async ({
  page,
}) => {
  await seedOnboardingCompletion(page, DEFAULT_MOCK_PUBKEY);
  await installMockBridge(page, {
    profileReadDelayMs: 3_000,
  });
  await page.goto("/");

  await expect(page.getByTestId("onboarding-gate")).toHaveCount(0);
  await expectHomeView(page);
});

test("first-run default workspace handoff gives immediate stepper feedback", async ({
  page,
}) => {
  // Use a blank-username identity so the profile has no display name and
  // the auto-skip logic does not fire — we need to stay in onboarding to
  // verify the stepper UX.
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await installMockBridge(
    page,
    {
      profileReadDelayMs: 2_000,
    },
    {
      relayWsUrl: "wss://default.example.com",
      skipOnboardingSeed: true,
      skipWorkspaceSeed: true,
    },
  );
  await page.goto("/");

  await expect(page.getByText("Welcome to Buzz")).toBeVisible();
  await page
    .getByRole("button", { name: "Continue with Block Inc. workspace" })
    .click();

  await page.waitForTimeout(80);
  await expect(page.getByRole("button", { name: "Connecting..." })).toHaveCount(
    0,
  );
  await expect(
    page.getByRole("button", { name: "Continue with Block Inc. workspace" }),
  ).toBeVisible();
  await expect(page.getByRole("progressbar")).toHaveAttribute(
    "aria-valuenow",
    "2",
  );
  await page.waitForTimeout(240);
  await expect(page.getByTestId("welcome-continue-nostr")).toBeVisible();
  await expect(page.getByRole("progressbar")).toHaveAttribute(
    "aria-valuenow",
    "2",
  );

  const nameInput = page.getByTestId("onboarding-display-name");
  await expect(nameInput).toHaveValue("");
  await expect(page.getByRole("progressbar")).toHaveAttribute(
    "aria-valuenow",
    "2",
  );
  await expect(nameInput).toHaveAttribute("autocomplete", "off");
  await expect(page.getByTestId("onboarding-back")).toBeVisible();
});

test("welcome can continue using an existing Nostr key", async ({ page }) => {
  await installMockBridge(page, undefined, {
    relayWsUrl: "wss://default.example.com",
    skipOnboardingSeed: true,
    skipWorkspaceSeed: true,
  });
  await page.goto("/");

  await page.getByTestId("welcome-continue-nostr").click();
  await expect(
    page.getByRole("heading", { name: "Use your existing key" }),
  ).toBeVisible();

  const importedNsec = nsecEncode(hexToBytes(TEST_IDENTITIES.alice.privateKey));
  await page.getByTestId("nostr-import-nsec-input").fill(importedNsec);
  await expect(page.getByTestId("nostr-import-npub-preview")).toBeVisible();
  await page.getByTestId("nostr-import-submit").click();

  // Alice already has a relay profile with a display name, so onboarding
  // auto-completes after the identity swap.
  await expect
    .poll(() =>
      page.evaluate(() => {
        const rawWorkspaces = window.localStorage.getItem("buzz-workspaces");
        const workspaces = rawWorkspaces
          ? (JSON.parse(rawWorkspaces) as Array<{ pubkey?: string }>)
          : [];
        return workspaces[0]?.pubkey ?? null;
      }),
    )
    .toBe(TEST_IDENTITIES.alice.pubkey);
  await expect(page.getByTestId("onboarding-gate")).toHaveCount(0);
  await expectHomeView(page);
});

test("welcome presents custom workspace setup as joining a workspace", async ({
  page,
}) => {
  await installMockBridge(page, undefined, {
    skipOnboardingSeed: true,
    skipWorkspaceSeed: true,
  });
  await page.goto("/");

  await expect(
    page.getByRole("button", { name: "Continue with Block Inc. workspace" }),
  ).toHaveCount(0);
  await page.getByRole("button", { name: "Join a workspace" }).click();

  await expect(
    page.getByRole("heading", { name: "Join a workspace" }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Join a workspace" }),
  ).toBeVisible();
});

test("identity fallback text does not count as a real onboarding name", async ({
  page,
}) => {
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  await expectIncompleteOnboarding(page);
  await expect(page.getByTestId("onboarding-next")).toBeDisabled();
});

test("avatar step uses an add-image placeholder before an avatar is chosen", async ({
  page,
}) => {
  await seedActiveIdentity(page, BLANK_AVATAR_PLACEHOLDER_IDENTITY);
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  await page.getByTestId("onboarding-display-name").fill("Morty QA");
  await page.getByTestId("onboarding-next").click();

  await expect(page.getByTestId("onboarding-page-avatar")).toBeVisible();
  const preview = page.getByTestId("onboarding-avatar-preview");
  await expect(preview).toBeVisible();
  await expect(preview).toHaveAttribute("aria-label", "Add a display image");
  await expect(preview).toHaveClass(/border-dashed/);
});

test("avatar step reveals preset backgrounds after the first emoji pick", async ({
  page,
}) => {
  await seedActiveIdentity(page, BLANK_AVATAR_EMOJI_IDENTITY);
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  await page.getByTestId("onboarding-display-name").fill("Morty QA");
  await page.getByTestId("onboarding-next").click();
  await expect(page.getByTestId("onboarding-page-avatar")).toBeVisible();

  await page.getByRole("tab", { name: "Emoji" }).click();

  const colorGridShell = page.getByTestId("onboarding-avatar-color-grid-shell");
  await expect(colorGridShell).toHaveAttribute("aria-hidden", "true");

  await selectFirstEmojiFromPicker(page);

  await expect(colorGridShell).toHaveAttribute("aria-hidden", "false");
  await expect(page.getByTestId("onboarding-avatar-color-grid")).toBeVisible();
  await expect(page.getByTestId("onboarding-avatar-preview")).not.toHaveCSS(
    "background-color",
    "rgb(255, 255, 255)",
  );
});

test("avatar step accepts an avatar URL before theme selection", async ({
  page,
}) => {
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  await page.getByTestId("onboarding-display-name").fill("Morty QA");
  await page.getByTestId("onboarding-next").click();
  await expect(page.getByTestId("onboarding-page-avatar")).toBeVisible();
  await page
    .getByTestId("onboarding-avatar-url")
    .fill("https://example.com/morty.png");

  const preview = page.getByTestId("onboarding-avatar-preview");
  await expect(preview).toBeVisible();
  const box = await preview.boundingBox();
  expect(box?.width).toBeCloseTo(192, 0);
  expect(box?.height).toBeCloseTo(192, 0);

  await page.getByTestId("onboarding-next").click();
  await expect(page.getByTestId("onboarding-page-theme")).toBeVisible();
  await page.getByTestId("onboarding-theme-option-github-light").click();
  await expect
    .poll(() => page.evaluate(() => window.localStorage.getItem("buzz-theme")))
    .toBe("github-light");
  await page.getByTestId("onboarding-next").click();
  await expect(page.getByTestId("onboarding-page-2")).toBeVisible();
  await expect(page.getByTestId("onboarding-runtime-goose")).toBeVisible();
});

test("failed avatar saves can continue without saving the avatar", async ({
  page,
}) => {
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await installMockBridge(page, {}, { skipOnboardingSeed: true });
  await page.goto("/");

  await page.getByTestId("onboarding-display-name").fill("Morty QA");
  await page.getByTestId("onboarding-next").click();
  await expect(page.getByTestId("onboarding-page-avatar")).toBeVisible();
  await page
    .getByTestId("onboarding-avatar-url")
    .fill("https://example.com/morty.png");
  await page.evaluate(() => {
    const testWindow = window as Window & {
      __BUZZ_E2E__?: { mock?: { profileUpdateError?: string } };
    };
    if (testWindow.__BUZZ_E2E__?.mock) {
      testWindow.__BUZZ_E2E__.mock.profileUpdateError =
        "Temporary avatar sync failure.";
    }
  });

  await page.getByTestId("onboarding-next").click();

  await expect(page.getByText("Temporary avatar sync failure.")).toBeVisible();
  await expect(
    page.getByTestId("onboarding-next-without-saving"),
  ).toBeVisible();
  await page.getByTestId("onboarding-next-without-saving").click();

  await expect(page.getByTestId("onboarding-page-theme")).toBeVisible();
});

test("theme step offers skip instead of going back", async ({ page }) => {
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  await page.getByTestId("onboarding-display-name").fill("Morty QA");
  await page.getByTestId("onboarding-next").click();
  await expect(page.getByTestId("onboarding-page-avatar")).toBeVisible();
  await page
    .getByTestId("onboarding-avatar-url")
    .fill("https://example.com/morty.png");
  await expect
    .poll(() => page.evaluate(() => window.localStorage.getItem("buzz-theme")))
    .toBe("github-light-default");
  await expect
    .poll(() =>
      page.evaluate(() => window.localStorage.getItem("buzz-accent-color")),
    )
    .toBe("neutral");
  await expect
    .poll(() =>
      page.evaluate(() => document.documentElement.classList.contains("light")),
    )
    .toBe(true);
  await page.getByTestId("onboarding-next").click();

  await expect(page.getByTestId("onboarding-page-theme")).toBeVisible();
  await expect
    .poll(() => page.evaluate(() => window.localStorage.getItem("buzz-theme")))
    .toBe("github-light-default");
  await expect
    .poll(() =>
      page.evaluate(() => window.localStorage.getItem("buzz-accent-color")),
    )
    .toBe("neutral");
  await expect(
    page.getByTestId("onboarding-theme-option-github-light-default"),
  ).toHaveAttribute("aria-pressed", "true");
  await expect(
    page.getByTestId("onboarding-accent-color-neutral"),
  ).toHaveAttribute("aria-pressed", "true");
  await page.getByTestId("onboarding-accent-color-blue").click();
  await expect
    .poll(() =>
      page.evaluate(() => window.localStorage.getItem("buzz-accent-color")),
    )
    .toBe("#3b82f6");
  await page.getByTestId("onboarding-accent-color-neutral").click();
  await expect
    .poll(() =>
      page.evaluate(() => window.localStorage.getItem("buzz-accent-color")),
    )
    .toBe("neutral");
  await expect(page.getByTestId("onboarding-back")).toHaveCount(0);
  await page.getByTestId("onboarding-skip").click();

  await expect(page.getByTestId("onboarding-page-2")).toBeVisible();
});

test("avatar upload rejects a file whose server-detected MIME is not an image", async ({
  page,
}) => {
  // Models a spoofed/blank picker MIME: the picked file claims to be an image
  // (passes the browser-side accept filter) but the shared generic upload path
  // returns a non-image descriptor. The post-upload backstop must reject it so
  // a non-image can't become an avatar (regression guard — the shared upload
  // path no longer rejects non-images server-side).
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await installMockBridge(
    page,
    {
      uploadDescriptors: [
        {
          url: `https://mock.relay/media/${"b".repeat(64)}.pdf`,
          sha256: "b".repeat(64),
          size: 4096,
          type: "application/pdf",
          uploaded: Math.floor(Date.now() / 1000),
          filename: "not-an-image.pdf",
        },
      ],
    },
    { skipOnboardingSeed: true },
  );
  await page.goto("/");

  await page.getByTestId("onboarding-display-name").fill("Morty QA");
  await page.getByTestId("onboarding-next").click();
  await expect(page.getByTestId("onboarding-page-avatar")).toBeVisible();
  await page.getByTestId("onboarding-avatar-input").setInputFiles({
    name: "looks-like.png",
    mimeType: "image/png",
    buffer: Buffer.from("not really a png"),
  });

  await expect(page.getByRole("alert")).toContainText(
    "Choose a PNG, JPG, GIF, or WebP image.",
  );
  await expect(page.getByTestId("onboarding-avatar-url")).toHaveValue("");
});

test("avatar upload accepts a file whose server-detected MIME is an image", async ({
  page,
}) => {
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  const url = `https://mock.relay/media/${"c".repeat(64)}.png`;
  await installMockBridge(
    page,
    {
      uploadDescriptors: [
        {
          url,
          sha256: "c".repeat(64),
          size: 2048,
          type: "image/png",
          uploaded: Math.floor(Date.now() / 1000),
        },
      ],
    },
    { skipOnboardingSeed: true },
  );
  await page.goto("/");

  await page.getByTestId("onboarding-display-name").fill("Morty QA");
  await page.getByTestId("onboarding-next").click();
  await expect(page.getByTestId("onboarding-page-avatar")).toBeVisible();
  await page.getByTestId("onboarding-avatar-input").setInputFiles({
    name: "avatar.png",
    mimeType: "image/png",
    buffer: Buffer.from("png bytes"),
  });

  await expect(page.getByTestId("onboarding-avatar-url")).toHaveValue("");
  await expect(
    page.getByTestId("onboarding-avatar-preview-fallback"),
  ).toHaveText("MQ");
  await expect(page.getByTestId("onboarding-avatar-error")).toHaveCount(0);
});

test("first-run onboarding keeps the shell hidden through setup and lands on Welcome after finish", async ({
  page,
}) => {
  await seedActiveIdentity(page, FIRST_RUN_ALICE);
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  await expect(page.getByTestId("onboarding-gate")).toBeVisible();
  await expect(page.getByTestId("onboarding-page-1")).toBeVisible();
  await expect(page.getByTestId("onboarding-display-name")).toHaveValue("");
  await expectNoHomeSeenEntries(page);

  await page.getByTestId("onboarding-display-name").fill("Alice");
  await continueToSetupPage(page);
  await expectShellHidden(page);
  await expect(page.getByTestId("onboarding-runtime-goose")).toBeVisible();
  await expectNoHomeSeenEntries(page);

  await page.getByTestId("onboarding-finish").click();
  await expect(page.getByTestId("onboarding-gate")).toHaveCount(0);
  await expectWelcomeView(page);
  await expectPrivateWelcomeChannel(page);
  await expectWelcomeGuideIntro(page);
});

test("first-run onboarding shows setup loading until Welcome bootstrap completes", async ({
  page,
}) => {
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await installMockBridge(
    page,
    { createManagedAgentDelayMs: 5_000 },
    { skipOnboardingSeed: true },
  );
  await page.goto("/");

  await page.getByTestId("onboarding-display-name").fill("Morty QA");
  await continueToSetupPage(page);
  await page.getByTestId("onboarding-finish").click();

  const loadingGate = page.getByTestId("app-loading-gate");
  await expect(page.getByTestId("onboarding-gate")).toHaveCount(0);
  await expect(loadingGate).toBeVisible();
  await expect(loadingGate).toContainText("Setting up your workspace...");
  await expect(
    loadingGate.getByTestId("setup-grainient-background"),
  ).toBeVisible();
  await expect(
    loadingGate.getByTestId("setup-grainient-background").locator("canvas"),
  ).toHaveCount(0);
  await expect
    .poll(async () =>
      loadingGate.evaluate((element) => {
        const wash = element.querySelector(".buzz-setup-grainient__wash");
        if (!(wash instanceof HTMLElement)) {
          return null;
        }

        const shellStyles = window.getComputedStyle(element);
        const washStyles = window.getComputedStyle(wash);
        const loadingText = element.querySelector(".buzz-setup-loading-text");
        const textStyles =
          loadingText instanceof HTMLElement
            ? window.getComputedStyle(loadingText)
            : null;
        return {
          animationName: washStyles.animationName,
          backgroundMatchesTheme:
            shellStyles.backgroundColor === washStyles.backgroundColor,
          textAvoidsHardcodedWhite: textStyles?.color !== "rgb(255, 255, 255)",
          usesRadialGradients:
            washStyles.backgroundImage.includes("radial-gradient"),
        };
      }),
    )
    .toEqual({
      animationName: "buzz-grainient-orbit",
      backgroundMatchesTheme: true,
      textAvoidsHardcodedWhite: true,
      usesRadialGradients: true,
    });
  await expect(loadingGate).not.toHaveClass(/buzz-onboarding-neutral-theme/);
  await expectShellHidden(page);
  await page.waitForTimeout(250);
  await expect(loadingGate).toBeVisible();

  await expectWelcomeView(page);
  await expectPrivateWelcomeChannel(page);
  await expectWelcomeGuideIntro(page);
});

test("existing relay profile with display name auto-skips onboarding without localStorage", async ({
  page,
}) => {
  // A user whose relay profile already has a display name should skip
  // onboarding even without the localStorage completion flag.
  await seedActiveIdentity(page, TEST_IDENTITIES.alice);
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  await expect(page.getByTestId("onboarding-gate")).toHaveCount(0);
  await expectHomeView(page);
});

test("onboarding can import an existing key when the workspace is already set up", async ({
  page,
}) => {
  // Workspace exists (default seed), but this identity has no profile yet,
  // so the app lands on the onboarding name step — Tyler's moved-laptop /
  // fresh-dev-instance case.
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  await expect(page.getByTestId("onboarding-display-name")).toHaveValue("");
  await page.getByTestId("onboarding-import-key").click();
  await expect(
    page.getByRole("heading", { name: "Use your existing key" }),
  ).toBeVisible();

  const importedNsec = nsecEncode(hexToBytes(TEST_IDENTITIES.alice.privateKey));
  await page.getByTestId("nostr-import-nsec-input").fill(importedNsec);
  await expect(page.getByTestId("nostr-import-npub-preview")).toBeVisible();
  await page.getByTestId("nostr-import-submit").click();

  // Identity swap remounts the flow; alice already has a relay profile with
  // a display name, so onboarding auto-completes and lands in the app.
  await expect(page.getByTestId("onboarding-gate")).toHaveCount(0);
  await expectHomeView(page);
});

test("completed onboarding backfills a missing private Welcome channel", async ({
  page,
}) => {
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await seedOnboardingCompletion(page, BLANK_TYLER_IDENTITY.pubkey);
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  await expect(page.getByTestId("onboarding-gate")).toHaveCount(0);
  await expectHomeView(page);
  await expect(page.getByTestId("channel-Welcome")).toBeVisible();
  await expectPrivateWelcomeChannel(page);
  await expectWelcomeGuideIntro(page, { expectVisible: false });
});

test("finishing onboarding creates and focuses a private Welcome channel for a new member", async ({
  page,
}) => {
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  await page.getByTestId("onboarding-display-name").fill("Morty QA");
  await continueToSetupPage(page);
  await page.getByTestId("onboarding-finish").click();

  await expectWelcomeView(page);
  await expect(page.getByTestId("channel-general")).toBeVisible();
  await expectPrivateWelcomeChannel(page);
  await expectWelcomeGuideIntro(page);
  await expectWelcomeComposerBannerCompletesAfterPersonaMention(page);
});

test("page 2 falls back to Doctor guidance when ACP tools are not installed", async ({
  page,
}) => {
  await seedActiveIdentity(page, FIRST_RUN_ALICE);
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [],
    },
    { skipOnboardingSeed: true },
  );
  await page.goto("/");

  await page.getByTestId("onboarding-display-name").fill("Alice");
  await continueToSetupPage(page);
  await expect(page.getByTestId("onboarding-acp-empty")).toBeVisible();
  await expect(
    page.getByText("Settings > Doctor", { exact: false }),
  ).toBeVisible();
});

test("initial profile read failures still hold incomplete users in onboarding", async ({
  page,
}) => {
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await installMockBridge(
    page,
    {
      profileReadError: "Temporary profile read failure.",
    },
    { skipOnboardingSeed: true },
  );
  await page.goto("/");

  await expectIncompleteOnboarding(page);
});

test("failed first profile saves can be skipped for the current session", async ({
  page,
}) => {
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await installMockBridge(
    page,
    {
      profileUpdateError: "Temporary profile sync failure.",
    },
    { skipOnboardingSeed: true },
  );
  await page.goto("/");

  await expect(page.getByTestId("onboarding-gate")).toBeVisible();
  await expect(page.getByTestId("onboarding-display-name")).toHaveValue("");

  await page.getByTestId("onboarding-display-name").fill("Morty QA");
  await page.getByTestId("onboarding-next").click();

  await expect(page.getByText("Temporary profile sync failure.")).toBeVisible();
  await page.getByTestId("onboarding-skip").click();

  await expect(page.getByTestId("onboarding-gate")).toHaveCount(0);
  await expectHomeView(page);
});

test("generic relay save failures use the generic reconnect card", async ({
  page,
}) => {
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await installMockBridge(
    page,
    {
      profileUpdateError: "relay unreachable: could not connect to relay",
    },
    { skipOnboardingSeed: true },
  );
  await page.goto("/");

  await page.getByTestId("onboarding-display-name").fill("Morty QA");
  await page.getByTestId("onboarding-next").click();

  await expect(
    page.getByTestId("onboarding-relay-reconnect-card"),
  ).toBeVisible();
  await expect(
    page.getByTestId("onboarding-relay-reconnect-card"),
  ).toContainText("Can't reach the relay");
  await expect(
    page.getByTestId("onboarding-relay-reconnect-card"),
  ).toContainText("Click to connect");
});

test("custom relay proxy sign-in failures use the generic reconnect card", async ({
  page,
}) => {
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await installMockBridge(
    page,
    {
      profileUpdateError:
        "relay unreachable: relay returned an unexpected HTML page (network sign-in?)",
    },
    { skipOnboardingSeed: true },
  );
  await page.goto("/");

  await page.getByTestId("onboarding-display-name").fill("Morty QA");
  await page.getByTestId("onboarding-next").click();

  await expect(
    page.getByTestId("onboarding-relay-reconnect-card"),
  ).toBeVisible();
  await expect(
    page.getByTestId("onboarding-relay-reconnect-card"),
  ).toContainText("Can't reach the relay");
});

test("relay access failures use the generic reconnect card", async ({
  page,
}) => {
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await installMockBridge(
    page,
    {
      profileUpdateError: "relay unreachable: 403 Forbidden",
    },
    { skipOnboardingSeed: true },
  );
  await page.goto("/");

  await page.getByTestId("onboarding-display-name").fill("Morty QA");
  await page.getByTestId("onboarding-next").click();

  await expect(
    page.getByTestId("onboarding-relay-reconnect-card"),
  ).toBeVisible();
  await expect(
    page.getByTestId("onboarding-relay-reconnect-card"),
  ).toContainText("Can't reach the relay");
});

test("dismissed relay save failures reappear on retry", async ({ page }) => {
  const relayError = "relay unreachable: could not connect to relay";
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await installMockBridge(
    page,
    {
      profileUpdateErrors: [relayError, relayError],
    },
    { skipOnboardingSeed: true },
  );
  await page.goto("/");

  await page.getByTestId("onboarding-display-name").fill("Morty QA");
  await page.getByTestId("onboarding-next").click();
  await expect(
    page.getByTestId("onboarding-relay-reconnect-card"),
  ).toBeVisible();

  await page.getByTestId("onboarding-relay-reconnect-card").hover();
  await page.getByTestId("onboarding-relay-reconnect-card-dismiss").click();
  await expect(page.getByTestId("onboarding-relay-reconnect-card")).toHaveCount(
    0,
  );

  await page.getByTestId("onboarding-next").click();
  await expect(
    page.getByTestId("onboarding-relay-reconnect-card"),
  ).toBeVisible();
});

test("existing relay profile with display name auto-completes onboarding", async ({
  page,
}) => {
  // A user whose relay profile already has a display name should skip
  // onboarding entirely — they've already set up their identity previously
  // (possibly on another machine or app data directory).
  await seedActiveIdentity(page, TEST_IDENTITIES.alice);
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  await expect(page.getByTestId("onboarding-gate")).toHaveCount(0);
  await expectHomeView(page);
});

test("membership denial can import a different invited key", async ({
  page,
}) => {
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await installMockBridge(
    page,
    {
      relayRole: null,
    },
    { skipOnboardingSeed: true },
  );
  await page.goto("/");

  await page.getByTestId("onboarding-display-name").fill("Morty QA");
  await page.getByTestId("onboarding-next").click();

  await expect(page.getByTestId("membership-denied")).toBeVisible();
  await page.getByTestId("membership-denied-change-key").click();

  const importedNsec = nsecEncode(hexToBytes(TEST_IDENTITIES.alice.privateKey));
  await page.getByTestId("membership-denied-nsec-input").fill(importedNsec);
  await expect(
    page.getByTestId("membership-denied-npub-preview"),
  ).toBeVisible();
  await page.getByTestId("membership-denied-import-key").click();

  // Alice already has a relay profile with a display name, so after the
  // identity swap the onboarding gate auto-completes.
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as Window & { __BUZZ_E2E_COMMANDS__?: string[] })
            .__BUZZ_E2E_COMMANDS__ ?? [],
      ),
    )
    .toEqual(expect.arrayContaining(["plugin:websocket|disconnect"]));
  await expect
    .poll(() =>
      page.evaluate((storageKey) => {
        const rawIdentity = window.localStorage.getItem(storageKey);
        const identity = rawIdentity
          ? (JSON.parse(rawIdentity) as { pubkey?: string })
          : null;
        return identity?.pubkey ?? null;
      }, E2E_IDENTITY_OVERRIDE_STORAGE_KEY),
    )
    .toBe(TEST_IDENTITIES.alice.pubkey);
  await expect(page.getByTestId("onboarding-gate")).toHaveCount(0);
  await expectHomeView(page);
});

test("onboarding relay reconnect — click shows Connected then auto-dismisses", async ({
  page,
}) => {
  // Produce the relay reconnect card via a relay-unreachable profile save error.
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await installMockBridge(
    page,
    {
      profileUpdateError: "relay unreachable: could not connect to relay",
    },
    { skipOnboardingSeed: true },
  );
  await page.goto("/");

  await page.getByTestId("onboarding-display-name").fill("Morty QA");
  await page.getByTestId("onboarding-next").click();

  const card = page.getByTestId("onboarding-relay-reconnect-card");
  await expect(card).toBeVisible();
  await expect(card).toContainText("Can't reach the relay");

  // Drive degraded before clicking so the card is in the expected error state.
  await setRelayConnectionState(page, "disconnected");

  // Click the reconnect button. The controller attempts reconnect; the mock
  // relay reconnects successfully (fast-path or via the state seam). Either
  // path should result in the card transitioning to Connected and then
  // auto-dismissing.
  await page.getByTestId("onboarding-reconnect-relay").click();

  // Drive connected to ensure the controller's connection-state path fires
  // and the component's hadActiveReconnectRef guard is satisfied.
  await setRelayConnectionState(page, "connected");

  await expect(card).toContainText("Connected", { timeout: 5_000 });
  await expect(card).not.toContainText("Can't reach the relay");

  // Auto-dismiss fires after ONBOARDING_CONNECTIVITY_SUCCESS_AUTO_DISMISS_MS
  // (2500ms). Allow generous headroom for CI.
  await expect(card).toBeHidden({ timeout: 10_000 });
});

test("onboarding relay reconnect — connected without a prior click does not show Connected", async ({
  page,
}) => {
  // Tests the hadActiveReconnectRef guard: if the relay transitions to
  // "connected" without the user having clicked the reconnect button, the card
  // must NOT show Connected. This guards against spurious success on initial
  // connection or background recovery with no user action.
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await installMockBridge(
    page,
    {
      profileUpdateError: "relay unreachable: could not connect to relay",
    },
    { skipOnboardingSeed: true },
  );
  await page.goto("/");

  await page.getByTestId("onboarding-display-name").fill("Morty QA");
  await page.getByTestId("onboarding-next").click();

  const card = page.getByTestId("onboarding-relay-reconnect-card");
  await expect(card).toBeVisible();

  // Drive to disconnected state (no reconnect click has happened).
  await setRelayConnectionState(page, "disconnected");

  // Drive to connected WITHOUT clicking — this would happen on a spontaneous
  // background recovery. The hadActiveReconnectRef guard must block markSuccess().
  await setRelayConnectionState(page, "connected");

  // Wait long enough for any spurious markSuccess() to fire, then assert the
  // card stayed in the error state.
  await page.waitForTimeout(500);
  await expect(card).toBeVisible();
  await expect(card).toContainText("Can't reach the relay");
  await expect(card).not.toContainText("Connected");
});
