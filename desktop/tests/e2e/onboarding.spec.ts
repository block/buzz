import { hexToBytes } from "@noble/hashes/utils.js";
import { expect, test, type Page } from "@playwright/test";
import { nsecEncode } from "nostr-tools/nip19";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const E2E_IDENTITY_OVERRIDE_STORAGE_KEY = "sprout:e2e-identity-override.v1";
const HOME_SEEN_STORAGE_KEY_PREFIX = "sprout-home-feed-seen.v1:";
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
const FIRST_RUN_KENNY = {
  ...TEST_IDENTITIES.tyler,
  username: "Kenny QA",
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
      storageKey: `sprout-onboarding-complete.v1:${pubkey}`,
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

async function expectHomeSeenCount(page: Page, expectedCount: number) {
  await expect
    .poll(async () => {
      return page.evaluate((prefix) => {
        const seenEntries = Object.entries(window.localStorage).filter(
          ([key]) => key.startsWith(prefix),
        );
        if (seenEntries.length === 0) {
          return 0;
        }

        const [, rawValue] = seenEntries[0];
        const parsed = JSON.parse(rawValue ?? "[]");
        return Array.isArray(parsed) ? parsed.length : 0;
      }, HOME_SEEN_STORAGE_KEY_PREFIX);
    })
    .toBe(expectedCount);
}

async function expectShellHidden(page: Page) {
  await expect(page.getByTestId("app-sidebar")).toHaveCount(0);
  await expect(page.getByTestId("chat-title")).toHaveCount(0);
}

async function expectHomeView(page: Page) {
  await expect(page.getByTestId("home-inbox-list")).toBeVisible();
}

async function getMockProfile(page: Page) {
  return page.evaluate(async () => {
    const invoke = (
      window as Window & {
        __SPROUT_E2E_INVOKE_MOCK_COMMAND__?: (
          command: string,
          payload?: Record<string, unknown>,
        ) => Promise<unknown>;
      }
    ).__SPROUT_E2E_INVOKE_MOCK_COMMAND__;
    if (!invoke) {
      throw new Error("Mock invoke bridge is unavailable.");
    }

    return (await invoke("get_profile")) as {
      avatar_url: string | null;
      display_name: string | null;
    };
  });
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
    .poll(() =>
      page.evaluate(() => window.localStorage.getItem("sprout-theme")),
    )
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
  await seedActiveIdentity(page, FIRST_RUN_KENNY);
  await installMockBridge(
    page,
    {
      profileReadDelayMs: 2_000,
    },
    { skipOnboardingSeed: true, skipWorkspaceSeed: true },
  );
  await page.goto("/");

  await expect(page.getByText("Welcome to Sprout")).toBeVisible();
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
  await expect(nameInput).toHaveValue("Kenny QA");
  await expect(page.getByRole("progressbar")).toHaveAttribute(
    "aria-valuenow",
    "2",
  );
  await expect(nameInput).toHaveAttribute("autocomplete", "off");
  await expect(page.getByTestId("onboarding-back")).toBeVisible();
});

test("welcome can continue using an existing Nostr key", async ({ page }) => {
  await installMockBridge(page, undefined, {
    skipOnboardingSeed: true,
    skipWorkspaceSeed: true,
  });
  await page.goto("/");

  await page.getByTestId("welcome-continue-nostr").click();
  await expect(
    page.getByRole("heading", { name: "Continue using Nostr" }),
  ).toBeVisible();

  const importedNsec = nsecEncode(hexToBytes(TEST_IDENTITIES.alice.privateKey));
  await page.getByTestId("welcome-nostr-nsec-input").fill(importedNsec);
  await expect(page.getByTestId("welcome-nostr-npub-preview")).toBeVisible();
  await page.getByTestId("welcome-nostr-submit").click();

  await expect(page.getByTestId("onboarding-display-name")).toHaveValue(
    "alice",
  );
  await expect
    .poll(() =>
      page.evaluate(() => {
        const rawWorkspaces = window.localStorage.getItem("sprout-workspaces");
        const workspaces = rawWorkspaces
          ? (JSON.parse(rawWorkspaces) as Array<{ pubkey?: string }>)
          : [];
        return workspaces[0]?.pubkey ?? null;
      }),
    )
    .toBe(TEST_IDENTITIES.alice.pubkey);
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
});

test("welcome presents custom workspace setup as joining a workspace", async ({
  page,
}) => {
  await installMockBridge(page, undefined, {
    skipOnboardingSeed: true,
    skipWorkspaceSeed: true,
  });
  await page.goto("/");

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
    .poll(() =>
      page.evaluate(() => window.localStorage.getItem("sprout-theme")),
    )
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
      __SPROUT_E2E__?: { mock?: { profileUpdateError?: string } };
    };
    if (testWindow.__SPROUT_E2E__?.mock) {
      testWindow.__SPROUT_E2E__.mock.profileUpdateError =
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
    .poll(() =>
      page.evaluate(() => window.localStorage.getItem("sprout-theme")),
    )
    .toBe("github-light-default");
  await expect
    .poll(() =>
      page.evaluate(() => window.localStorage.getItem("sprout-accent-color")),
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
    .poll(() =>
      page.evaluate(() => window.localStorage.getItem("sprout-theme")),
    )
    .toBe("github-light-default");
  await expect
    .poll(() =>
      page.evaluate(() => window.localStorage.getItem("sprout-accent-color")),
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
      page.evaluate(() => window.localStorage.getItem("sprout-accent-color")),
    )
    .toBe("#3b82f6");
  await page.getByTestId("onboarding-accent-color-neutral").click();
  await expect
    .poll(() =>
      page.evaluate(() => window.localStorage.getItem("sprout-accent-color")),
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

  await expect(page.getByTestId("onboarding-avatar-url")).toHaveValue(url);
  await expect(page.getByTestId("onboarding-avatar-error")).toHaveCount(0);
});

test("first-run onboarding keeps the shell hidden through setup and only marks Home seen after finish", async ({
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
  await expectHomeView(page);
  await expectHomeSeenCount(page, 2);
});

test("existing relay profile prefills the name step without localStorage completion", async ({
  page,
}) => {
  await seedActiveIdentity(page, TEST_IDENTITIES.alice);
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  await expect(page.getByTestId("onboarding-gate")).toBeVisible();
  await expectShellHidden(page);
  await expect(page.getByTestId("onboarding-display-name")).toHaveValue(
    "alice",
  );
  await expect(page.getByTestId("onboarding-next")).toBeEnabled();
  await expect(page.getByTestId("onboarding-back")).toHaveCount(0);
});

test("finishing onboarding auto-joins the #general channel for a new member", async ({
  page,
}) => {
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  await page.getByTestId("onboarding-display-name").fill("Morty QA");
  await continueToSetupPage(page);
  await page.getByTestId("onboarding-finish").click();

  await expectHomeView(page);
  await expect(page.getByTestId("channel-general")).toBeVisible();
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

test("failed saved profile saves can continue without retrying the display name", async ({
  page,
}) => {
  await seedActiveIdentity(page, TEST_IDENTITIES.alice);
  await installMockBridge(
    page,
    {
      profileUpdateError: "Temporary profile sync failure.",
    },
    { skipOnboardingSeed: true },
  );
  await page.goto("/");

  await expect(page.getByTestId("onboarding-display-name")).toHaveValue(
    "alice",
  );
  await page.getByTestId("onboarding-display-name").fill("Alice Draft");
  await page.getByTestId("onboarding-next").click();

  await expect(page.getByText("Temporary profile sync failure.")).toBeVisible();
  await page.getByTestId("onboarding-next-without-saving").click();

  await expect(page.getByTestId("onboarding-page-avatar")).toBeVisible();
  const avatarUrl = "https://example.com/alice-onboarding-avatar.png";
  await page.getByTestId("onboarding-avatar-url").fill(avatarUrl);
  await page.getByTestId("onboarding-next").click();

  await expect(page.getByTestId("onboarding-page-theme")).toBeVisible();
  await expect(await getMockProfile(page)).toMatchObject({
    avatar_url: avatarUrl,
    display_name: "alice",
  });
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

  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as Window & { __SPROUT_E2E_COMMANDS__?: string[] })
            .__SPROUT_E2E_COMMANDS__ ?? [],
      ),
    )
    .toEqual(expect.arrayContaining(["plugin:websocket|disconnect"]));
  await expect(page.getByTestId("onboarding-page-1")).toBeVisible();
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
  await page.getByTestId("onboarding-next").click();

  await expect(page.getByTestId("onboarding-page-avatar")).toBeVisible();
});
