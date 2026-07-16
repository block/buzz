import { hexToBytes } from "@noble/hashes/utils.js";
import { expect, test, type Locator, type Page } from "@playwright/test";
import { npubEncode, nsecEncode } from "nostr-tools/nip19";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import {
  E2E_IDENTITY_OVERRIDE_STORAGE_KEY,
  seedActiveIdentity,
} from "../helpers/onboarding";

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
  title: string,
) {
  const iconBox = await action.locator("svg").first().boundingBox();
  const titleBox = await action.getByText(title, { exact: true }).boundingBox();
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
    "Create a channel",
  );
  await expect(
    page
      .getByTestId("welcome-intro-action-create-channel")
      .getByText("Create a channel", { exact: true }),
  ).toHaveCSS("white-space", "normal");
  await expect(
    page.getByTestId("welcome-intro-action-create-agent"),
  ).toBeVisible();
  await expectWiderThanTall(
    page.getByTestId("welcome-intro-action-create-agent"),
  );
  await expectIntroActionIconStackedAboveTitle(
    page.getByTestId("welcome-intro-action-create-agent"),
    "Create an agent",
  );
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

async function completeProfileOnboarding(page: Page) {
  await page.getByTestId("onboarding-next").click();
  await expect(page.getByTestId("onboarding-page-avatar")).toBeVisible();
  await page
    .getByTestId("onboarding-avatar-url")
    .fill("https://example.com/onboarding-avatar.png");
  await page.getByTestId("onboarding-next").click();
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

test("first-community choices expose npub and invite input", async ({
  page,
}) => {
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await page.addInitScript((pubkey) => {
    window.localStorage.setItem(
      `buzz-machine-onboarding-complete.v2:${pubkey}`,
      "true",
    );
  }, BLANK_TYLER_IDENTITY.pubkey);
  await installMockBridge(page, undefined, {
    relayWsUrl: "wss://default.example.com",
    skipOnboardingSeed: true,
    skipCommunitySeed: true,
  });
  await page.goto("/");

  await expect(
    page.getByRole("button", { name: "Join a community" }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "I have an invite link" }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Create a community" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Create a community" }).click();
  await expect
    .poll(() =>
      page.evaluate(() => {
        const log = (
          window as Window & {
            __BUZZ_E2E_COMMAND_LOG__?: Array<{
              command: string;
              payload: unknown;
            }>;
          }
        ).__BUZZ_E2E_COMMAND_LOG__;
        return log?.find((entry) => entry.command === "plugin:opener|open_url")
          ?.payload;
      }),
    )
    .toMatchObject({ url: "https://buzz.xyz" });

  await page.getByRole("button", { name: "Join a community" }).click();
  await expect(page.getByTestId("welcome-join-npub")).toHaveText(
    npubEncode(BLANK_TYLER_IDENTITY.pubkey),
  );
  await page.getByRole("button", { name: "Back" }).click();

  await page.getByRole("button", { name: "I have an invite link" }).click();
  await expect(
    page.getByRole("heading", { name: "I have an invite link" }),
  ).toBeVisible();
  await expect(page.getByTestId("invite-redeem-input")).toBeVisible();
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

// Regression test for the H2 predicate fix (PR #1508).
// A blank first-run identity (no kind:0 event on the relay) must see
// onboarding even when the mock bridge returns display_name: "" — the gate
// must depend on `hasProfileEvent`, not on `typeof displayName === "string"`.
test("first-run blank identity with no profile event sees onboarding", async ({
  page,
}) => {
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  // Do NOT seed searchProfiles for tyler's pubkey, so ensureMockProfile
  // constructs a synthesised profile with has_profile_event: false.
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  await expectIncompleteOnboarding(page);
});

// Regression test for the H2 predicate fix (PR #1508).
// A returning user who has a real kind:0 profile event with an empty
// display_name must skip onboarding — they are already onboarded.
test("returning user with blank display name and real profile event skips onboarding", async ({
  page,
}) => {
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  // Seed tyler's pubkey into searchProfiles with an empty displayName.
  // seedMockSearchProfiles stores this into mockProfiles with
  // has_profile_event: true, simulating a real kind:0 event with no name.
  await installMockBridge(
    page,
    {
      searchProfiles: [
        {
          pubkey: TEST_IDENTITIES.tyler.pubkey,
          displayName: "",
        },
      ],
    },
    { skipOnboardingSeed: true },
  );
  await page.goto("/");

  // Profile event exists → onboarding is skipped, app renders.
  await expect(page.getByTestId("onboarding-gate")).toHaveCount(0);
  await expectHomeView(page);
});

// Regression test for the cache-seed defect (PR #1508 CRITICAL fix).
// Sequence: no-event profile fetched and cached with hasProfileEvent absent →
// reload with cache present → onboarding must still show. Previously the
// initialData seed hardcoded hasProfileEvent: true for any updatedAt > 0
// entry, reopening the original bug on the second app load.
test("no-event profile cached then reloaded still sees onboarding", async ({
  page,
}) => {
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  // Seed a stale v1 cache entry WITHOUT hasProfileEvent (simulating a cache
  // written by the old code path or a no-event fallback). updatedAt > 0 so
  // the seed is eligible, but hasProfileEvent is absent → conservative false.
  const SELF_PROFILE_CACHE_KEY = `buzz-self-profile.v1:ws://localhost:3000:${TEST_IDENTITIES.tyler.pubkey}`;
  await page.addInitScript(
    ({ key, cache }) => {
      window.localStorage.setItem(key, JSON.stringify(cache));
    },
    {
      key: SELF_PROFILE_CACHE_KEY,
      cache: {
        version: 1,
        displayName: null,
        avatarUrl: null,
        avatarDataUrl: null,
        updatedAt: 1_700_000_000_000,
        // hasProfileEvent deliberately absent — legacy/no-event entry.
      },
    },
  );
  // No profile event on the relay either — ensureMockProfile uses false.
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  // Cache seed must NOT promote hasProfileEvent to true. Onboarding shows.
  await expectIncompleteOnboarding(page);
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

test("avatar step accepts an avatar URL before completing onboarding", async ({
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
  await expect(page.getByTestId("onboarding-gate")).toHaveCount(0);
  await expectWelcomeView(page);
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

  await expect(page.getByTestId("onboarding-gate")).toHaveCount(0);
  await expectWelcomeView(page);
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

test("first-run onboarding keeps the shell hidden and lands on Welcome after profile setup", async ({
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
  await completeProfileOnboarding(page);
  await expect(page.getByTestId("onboarding-gate")).toHaveCount(0);
  await expectWelcomeView(page);
  await expectPrivateWelcomeChannel(page);
  await expectWelcomeGuideIntro(page);
});

function retryToast(page: Page, title: string) {
  return page
    .locator("[data-sonner-toast][data-removed='false']")
    .filter({ hasText: title });
}

async function retryToastAction(
  page: Page,
  { command, title }: { command: string; title: string },
) {
  const activeToast = retryToast(page, title);
  await expect(activeToast).toBeVisible();
  await expect(
    activeToast.getByRole("button", { name: "Retry" }),
  ).toBeVisible();

  const commandCountBeforeRetry = await page.evaluate(
    (retryCommand) =>
      (
        window as Window & {
          __BUZZ_E2E_COMMANDS__?: string[];
        }
      ).__BUZZ_E2E_COMMANDS__?.filter((entry) => entry === retryCommand)
        .length ?? 0,
    command,
  );
  await activeToast
    .getByRole("button", { name: "Retry" })
    .evaluate((button) => (button as HTMLButtonElement).click());

  await expect
    .poll(() =>
      page.evaluate(
        ({ retryCommand }) =>
          (
            window as Window & {
              __BUZZ_E2E_COMMANDS__?: string[];
            }
          ).__BUZZ_E2E_COMMANDS__?.filter((entry) => entry === retryCommand)
            .length ?? 0,
        { retryCommand: command, minimum: commandCountBeforeRetry + 1 },
      ),
    )
    .toBeGreaterThanOrEqual(commandCountBeforeRetry + 1);
}

async function expectRetryFailureRecreatesActionableToast(
  page: Page,
  { command, error, title }: { command: string; error: string; title: string },
) {
  const activeToast = retryToast(page, title);
  await expect(activeToast).toContainText(error);

  await retryToastAction(page, { command, title });

  await expect(activeToast).toHaveCount(1);
  await expect(activeToast).toContainText(error);
  await expect(
    activeToast.getByRole("button", { name: "Retry" }),
  ).toBeVisible();
}

async function expectRetrySuccessDismissesToast(
  page: Page,
  { command, title }: { command: string; title: string },
) {
  const activeToast = retryToast(page, title);

  await retryToastAction(page, { command, title });

  await expect(activeToast).toHaveCount(0);
}

test("failed Welcome and general retries recreate actionable toasts", async ({
  page,
}) => {
  const welcomeError = "Mock Welcome create failed.";
  const generalError = "Mock general join failed.";
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await installMockBridge(
    page,
    {
      createChannelErrors: [welcomeError, welcomeError],
      joinChannelErrors: [generalError, generalError],
    },
    { skipOnboardingSeed: true },
  );
  await page.goto("/");

  await page.getByTestId("onboarding-display-name").fill("Morty QA");
  await completeProfileOnboarding(page);

  await expectRetryFailureRecreatesActionableToast(page, {
    command: "create_channel",
    error: welcomeError,
    title: "Couldn't set up the Welcome channel",
  });
  await expectRetryFailureRecreatesActionableToast(page, {
    command: "join_channel",
    error: generalError,
    title: "Couldn't join #general",
  });
});

test("successful Welcome and general retries clear their actionable toasts", async ({
  page,
}) => {
  const welcomeError = "Mock Welcome create failed.";
  const generalError = "Mock general join failed.";
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await installMockBridge(
    page,
    {
      createChannelErrors: [welcomeError],
      joinChannelErrors: [generalError],
    },
    { skipOnboardingSeed: true },
  );
  await page.goto("/");

  await page.getByTestId("onboarding-display-name").fill("Morty QA");
  await completeProfileOnboarding(page);

  await expectRetrySuccessDismissesToast(page, {
    command: "create_channel",
    title: "Couldn't set up the Welcome channel",
  });
  await expectRetrySuccessDismissesToast(page, {
    command: "join_channel",
    title: "Couldn't join #general",
  });
  await expectWelcomeView(page);
  await expect(page.getByTestId("channel-general")).toBeVisible();
});

test("first-run onboarding shows setup loading until Welcome bootstrap completes", async ({
  page,
}) => {
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await installMockBridge(
    page,
    { createManagedAgentDelayMs: 9_000 },
    { skipOnboardingSeed: true },
  );
  await page.goto("/");

  await page.getByTestId("onboarding-display-name").fill("Morty QA");
  await completeProfileOnboarding(page);

  const loadingGate = page.getByTestId("app-loading-gate");
  await expect(page.getByTestId("onboarding-gate")).toHaveCount(0);
  await expect(loadingGate).toBeVisible();
  await expect(loadingGate).toContainText("Setting up your community...");

  // The boot gate is the theme-adaptive grainient with the flapping Buzz bee
  // as its hero. The mark must paint complete on the FIRST frame — a blank
  // gate reads as "nothing is loading" — so nothing about it may depend on
  // SMIL/scripted animation (<animate> count stays 0); the wing flap is pure
  // CSS on HTML-level wing layers so it keeps running on the compositor even
  // while boot work hogs the main thread.
  await expect(
    loadingGate.getByTestId("setup-grainient-background"),
  ).toBeVisible();
  const mark = loadingGate.locator(".buzz-mark");
  await expect(mark).toBeVisible();
  const gateTreatment = await loadingGate.evaluate((element) => {
    const markElement = element.querySelector(".buzz-mark");
    const markSvgs = markElement
      ? Array.from(markElement.querySelectorAll("svg"))
      : [];
    const wing = element.querySelector(".bee-wing");
    const wingStyles =
      wing instanceof SVGElement ? window.getComputedStyle(wing) : null;
    const wash = element.querySelector(".buzz-setup-grainient__wash");
    const washStyles =
      wash instanceof HTMLElement ? window.getComputedStyle(wash) : null;
    return {
      animateElementCount: element.querySelectorAll("animate").length,
      // The document itself must not flash white before the gate mounts
      // (inline <style> in index.html; black fallback when no cached theme).
      documentBackgroundColor: window.getComputedStyle(document.documentElement)
        .backgroundColor,
      grainientAnimation: washStyles?.animationName,
      grainientUsesRadialGradients:
        washStyles?.backgroundImage.includes("radial-gradient"),
      markSvgsUseCurrentColor:
        markSvgs.length > 0 &&
        markSvgs.every((svg) => svg.getAttribute("fill") === "currentColor"),
      wingFlapAnimation: wingStyles?.animationName,
      wingFlapRunning: wingStyles?.animationPlayState,
    };
  });
  expect(gateTreatment).toEqual({
    animateElementCount: 0,
    documentBackgroundColor: "rgb(0, 0, 0)",
    grainientAnimation: "buzz-grainient-orbit",
    grainientUsesRadialGradients: true,
    markSvgsUseCurrentColor: true,
    wingFlapAnimation: "bee-wing-left-flap",
    wingFlapRunning: "running",
  });
  await expect(loadingGate).not.toHaveClass(/buzz-onboarding-neutral-theme/);
  await expectShellHidden(page);
  await page.waitForTimeout(250);
  await expect(loadingGate).toBeVisible();
  await expect(mark).toBeVisible();

  await expectWelcomeView(page);
  await expectPrivateWelcomeChannel(page);
  await expectWelcomeGuideIntro(page);
});

test("existing relay profile with display name auto-skips onboarding without localStorage", async ({
  page,
}) => {
  // A user whose relay profile already has a display name should skip
  // onboarding even without the localStorage completion flag.
  // Seed alice's pubkey into searchProfiles so seedMockSearchProfiles writes
  // has_profile_event: true into mockProfiles — the harness-intended mechanism
  // for simulating a returning user with a real kind:0 event. Do NOT use the
  // static mockProfiles seed (removed in PR #1508); that path collides with
  // FIRST_RUN_ALICE (same pubkey) and breaks the first-run onboarding specs.
  await seedActiveIdentity(page, TEST_IDENTITIES.alice);
  await installMockBridge(
    page,
    {
      searchProfiles: [
        { pubkey: TEST_IDENTITIES.alice.pubkey, displayName: "alice" },
      ],
    },
    { skipOnboardingSeed: true },
  );
  await page.goto("/");

  await expect(page.getByTestId("onboarding-gate")).toHaveCount(0);
  await expectHomeView(page);
});

test("onboarding can import an existing key when the community is already set up", async ({
  page,
}) => {
  // Community exists (default seed), but this identity has no profile yet,
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
  await completeProfileOnboarding(page);

  await expectWelcomeView(page);
  await expect(page.getByTestId("channel-general")).toBeVisible();
  await expectPrivateWelcomeChannel(page);
  await expectWelcomeGuideIntro(page);
  await expectWelcomeComposerBannerCompletesAfterPersonaMention(page);
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

test("community access failures use the generic reconnect card", async ({
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
  // Seed alice's pubkey into searchProfiles so seedMockSearchProfiles writes
  // has_profile_event: true into mockProfiles — the harness-intended mechanism
  // for simulating a returning user with a real kind:0 event. Do NOT use the
  // static mockProfiles seed (removed in PR #1508); that path collides with
  // FIRST_RUN_ALICE (same pubkey) and breaks the first-run onboarding specs.
  await seedActiveIdentity(page, TEST_IDENTITIES.alice);
  await installMockBridge(
    page,
    {
      searchProfiles: [
        { pubkey: TEST_IDENTITIES.alice.pubkey, displayName: "alice" },
      ],
    },
    { skipOnboardingSeed: true },
  );
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
  // The identity swap must tear down the old relay socket. There is no
  // `plugin:websocket|disconnect` command in tauri-plugin-websocket — closing
  // is a Close frame sent through `plugin:websocket|send`.
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (
            window as Window & {
              __BUZZ_E2E_COMMAND_PAYLOADS__?: Array<{
                command: string;
                payload: unknown;
              }>;
            }
          ).__BUZZ_E2E_COMMAND_PAYLOADS__?.some(
            (entry) =>
              entry.command === "plugin:websocket|send" &&
              (entry.payload as { message?: { type?: string } })?.message
                ?.type === "Close",
          ) ?? false,
      ),
    )
    .toBe(true);
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

test("membership denied shows all four affordances and change-community edits non-destructively", async ({
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

  // Fill the display name and advance — membership check triggers denial.
  await page.getByTestId("onboarding-display-name").fill("Morty QA");
  await page.getByTestId("onboarding-next").click();

  // Membership-denied screen renders with all four affordances.
  const denied = page.getByTestId("membership-denied");
  await expect(denied).toBeVisible();
  await expect(denied.getByRole("button", { name: "Try again" })).toBeVisible();
  await expect(denied.getByRole("button", { name: "Back" })).toBeVisible();
  await expect(
    denied.getByRole("button", { name: "Change community" }),
  ).toBeVisible();
  await expect(page.getByTestId("membership-denied-change-key")).toBeVisible();
  await expect(
    page.getByTestId("membership-denied-redeem-invite"),
  ).toBeVisible();

  // Click "Change community" → the overlay opens.
  await denied.getByRole("button", { name: "Change community" }).click();
  const overlay = page.getByTestId("community-change-overlay");
  await expect(overlay).toBeVisible();

  // Change the relay URL to a new one. The probe will time out for a fake URL
  // so we wait for the "Use anyway" button.
  await overlay
    .locator("#community-edit-url")
    .fill("wss://new-relay.example.com");
  await overlay.getByRole("button", { name: "Save changes" }).click();

  // The fields are frozen while the probe is pending, so the saved URL and
  // any warning cannot get out of sync with a subsequent edit.
  await expect(overlay.locator("#community-edit-url")).toBeDisabled();
  await expect(overlay.locator("#community-edit-name")).toBeDisabled();
  await expect(
    overlay.getByRole("button", { name: "Use anyway" }),
  ).toBeVisible();
  await overlay.getByRole("button", { name: "Use anyway" }).click();

  // The community update triggers a remount (reinitKey bump). The persisted
  // community should now point to the new relay URL.
  await expect
    .poll(() =>
      page.evaluate(() => {
        const raw = window.localStorage.getItem("buzz-communities");
        const communities = raw
          ? (JSON.parse(raw) as Array<{ relayUrl?: string }>)
          : [];
        return communities[0]?.relayUrl ?? null;
      }),
    )
    .toBe("wss://new-relay.example.com");

  // Identity was NOT wiped — the override storage key is still intact.
  await expect
    .poll(() =>
      page.evaluate((storageKey) => {
        return window.localStorage.getItem(storageKey) !== null;
      }, E2E_IDENTITY_OVERRIDE_STORAGE_KEY),
    )
    .toBe(true);
});

test("cancel from profile Back preserves drafts and denied Back returns to interrupted page", async ({
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

  // --- Profile step: Back opens community-change overlay ---
  const nameInput = page.getByTestId("onboarding-display-name");
  await nameInput.fill("Morty QA");
  await page.getByTestId("onboarding-back").click();

  // The community change overlay should open.
  const overlay = page.getByTestId("community-change-overlay");
  await expect(overlay).toBeVisible();

  // Cancel the overlay — profile should still be visible with the name intact.
  await overlay.getByRole("button", { name: "Cancel" }).click();
  await expect(overlay).toHaveCount(0);
  await expect(page.getByTestId("onboarding-page-1")).toBeVisible();
  await expect(nameInput).toHaveValue("Morty QA");

  // --- Profile → membership denied → Back returns to profile ---
  // Advance triggers membership check → denied (relayRole is null).
  await page.getByTestId("onboarding-next").click();
  await expect(page.getByTestId("membership-denied")).toBeVisible();

  // Press Back on the denied screen — should return to the profile page
  // (deniedFromPage = "profile") with the name draft intact.
  await page
    .getByTestId("membership-denied")
    .getByRole("button", { name: "Back" })
    .click();
  await expect(page.getByTestId("onboarding-page-1")).toBeVisible();
  await expect(nameInput).toHaveValue("Morty QA");
});

test("denied on relay A then paste relay B invite URL switches community to B", async ({
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

  // Record the initial relay URL (relay A).
  const initialRelayUrl = await page.evaluate(() => {
    const raw = window.localStorage.getItem("buzz-communities");
    const communities = raw
      ? (JSON.parse(raw) as Array<{ relayUrl?: string }>)
      : [];
    return communities[0]?.relayUrl ?? null;
  });
  expect(initialRelayUrl).not.toBeNull();

  // Fill name, advance → denied on relay A.
  await page.getByTestId("onboarding-display-name").fill("Morty QA");
  await page.getByTestId("onboarding-next").click();
  await expect(page.getByTestId("membership-denied")).toBeVisible();

  // Intercept the claimInvite POST to relay B so it succeeds.
  const relayBUrl = "wss://relay-b.example.com";
  const relayBHttpUrl = "https://relay-b.example.com";
  await page.route(`${relayBHttpUrl}/api/invites/claim`, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        status: "joined",
        community_id: "mock-community",
        host: "relay-b.example.com",
        role: "member",
      }),
    });
  });

  // Click "Have an invite?" and enter an HTTPS invite URL for relay B.
  await page.getByTestId("membership-denied-redeem-invite").click();
  await page
    .getByTestId("invite-redeem-input")
    .fill(`${relayBHttpUrl}/invite/test-invite-code`);
  await page.getByTestId("invite-redeem-submit").click();

  // After successful claim, relay B is added and becomes active; relay A remains
  // in the community list for future switching.
  await expect
    .poll(() =>
      page.evaluate(() => {
        const raw = window.localStorage.getItem("buzz-communities");
        const activeCommunityId = window.localStorage.getItem(
          "buzz-active-community-id",
        );
        const communities = raw
          ? (JSON.parse(raw) as Array<{ id?: string; relayUrl?: string }>)
          : [];
        return communities.find(({ id }) => id === activeCommunityId)?.relayUrl;
      }),
    )
    .toBe(relayBUrl);
});
