import { expect, test, type Page } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const E2E_IDENTITY_OVERRIDE_STORAGE_KEY = "sprout:e2e-identity-override.v1";
const HOME_SEEN_STORAGE_KEY_PREFIX = "sprout-home-feed-seen.v1:";
const DEFAULT_MOCK_PUBKEY = "deadbeef".repeat(8);
const BLANK_TYLER_IDENTITY = {
  ...TEST_IDENTITIES.tyler,
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

async function expectIncompleteOnboarding(page: Page) {
  await expect(page.getByTestId("onboarding-gate")).toBeVisible();
  await expectShellHidden(page);
  await expect(page.getByTestId("onboarding-page-1")).toBeVisible();
  await expect(page.getByTestId("onboarding-display-name")).toHaveValue("");
}

async function continueToSetupPage(page: Page) {
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

test("identity fallback text does not count as a real onboarding name", async ({
  page,
}) => {
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  await expectIncompleteOnboarding(page);
  await expect(page.getByTestId("onboarding-avatar-upload")).toHaveText(
    "Drop an image or browse",
  );
  await expect(page.getByTestId("onboarding-avatar-url")).toHaveValue("");
  await expect(page.getByTestId("onboarding-next")).toBeDisabled();
});

test("page 1 accepts an avatar URL as the secondary avatar path", async ({
  page,
}) => {
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  await page.getByTestId("onboarding-display-name").fill("Morty QA");
  await page
    .getByTestId("onboarding-avatar-url")
    .fill("https://example.com/morty.png");

  const preview = page.getByTestId("onboarding-avatar-preview");
  await expect(preview).toBeVisible();
  const box = await preview.boundingBox();
  expect(box?.width).toBeCloseTo(80, 0);
  expect(box?.height).toBeCloseTo(80, 0);

  await continueToSetupPage(page);
  await expect(page.getByTestId("onboarding-runtime-goose")).toBeVisible();
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

  await page.getByTestId("onboarding-avatar-input").setInputFiles({
    name: "looks-like.png",
    mimeType: "image/png",
    buffer: Buffer.from("not really a png"),
  });

  await expect(page.getByTestId("onboarding-avatar-error")).toContainText(
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

  await page.getByTestId("onboarding-avatar-input").setInputFiles({
    name: "avatar.png",
    mimeType: "image/png",
    buffer: Buffer.from("png bytes"),
  });

  await expect(page.getByTestId("onboarding-avatar-url")).toHaveValue(url);
  await expect(page.getByTestId("onboarding-avatar-error")).toHaveCount(0);
});

test("first-run onboarding keeps the shell hidden through both pages and only marks Home seen after finish", async ({
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

test("existing relay profile auto-skips onboarding without localStorage completion", async ({
  page,
}) => {
  await seedActiveIdentity(page, TEST_IDENTITIES.alice);
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  await expect(page.getByTestId("onboarding-gate")).toHaveCount(0);
  await expectHomeView(page);
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
