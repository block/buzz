import { expect, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { seedActiveIdentity } from "../helpers/onboarding";

// Community deep links that arrive before machine onboarding complete are
// drained from Rust into a persisted transaction and acknowledged immediately.
// Invite claiming waits until setup finishes and the final identity is known.

const DEFAULT_MOCK_PUBKEY = "deadbeef".repeat(8);
const WELCOME_FAILURE_PUBKEY = TEST_IDENTITIES.tyler.pubkey;
const TRANSACTION_STORAGE_KEY = "buzz-community-onboarding-transaction.v1";
const COMMUNITY_RELAY_URL = "wss://hive.example.com";

const PENDING_JOIN_LINK = {
  id: "dl-join-1",
  kind: "join" as const,
  relayUrl: "wss://hive.example.com",
  code: "abc.def",
};

const PENDING_CONNECT_LINK = {
  id: "dl-connect-1",
  kind: "connect" as const,
  relayUrl: "wss://hive.example.com",
  code: null,
};

const PENDING_ADD_COMMUNITY_LINK = {
  id: "dl-add-community-1",
  kind: "add-community" as const,
  relayUrl: "wss://acme.communities.buzz.xyz",
  code: null,
  name: "Acme Team",
};

const SECOND_PENDING_ADD_COMMUNITY_LINK = {
  id: "dl-add-community-2",
  kind: "add-community" as const,
  relayUrl: "wss://beta.communities.buzz.xyz",
  code: null,
  name: "Beta Team",
};

test("join deep link is acknowledged without claiming before setup", async ({
  page,
}) => {
  let claimCalls = 0;
  await page.route("**/api/invites/claim", async (route) => {
    claimCalls++;
    await route.abort();
  });
  await installMockBridge(
    page,
    { pendingCommunityDeepLinks: [PENDING_JOIN_LINK] },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");

  const gate = page.getByTestId("pending-invite-gate");
  await expect(gate).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Opening community link" }),
  ).toBeVisible();
  await page.getByTestId("pending-invite-continue").click();
  await expect(gate).toHaveCount(0);
  await expect(page.getByTestId("machine-onboarding-gate")).toBeVisible();
  expect(claimCalls).toBe(0);
  await expect
    .poll(() =>
      page.evaluate(
        (key) => window.localStorage.getItem(key),
        TRANSACTION_STORAGE_KEY,
      ),
    )
    .toContain('"stage":"claiming"');
});

test("connect deep link shows a static acknowledgment during setup", async ({
  page,
}) => {
  // No invite code means nothing to confirm against the relay — the gate
  // acknowledges the link and waits for the user instead of auto-advancing.
  await installMockBridge(
    page,
    { pendingCommunityDeepLinks: [PENDING_CONNECT_LINK] },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");

  const gate = page.getByTestId("pending-invite-gate");
  await expect(gate).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Opening community link" }),
  ).toBeVisible();
  await expect(gate).toContainText("hive");

  // Continue setup dismisses the gate but keeps the transaction: the
  // connect resumes in CommunityOnboardingFlow after machine setup.
  await page.getByTestId("pending-invite-continue").click();
  await expect(gate).toHaveCount(0);
  await expect(page.getByTestId("machine-onboarding-gate")).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        (key) => window.localStorage.getItem(key),
        TRANSACTION_STORAGE_KEY,
      ),
    )
    .toContain('"acknowledged":true');
});

test("add-community deep link starts onboarding when no community is configured", async ({
  page,
}) => {
  // profileReadError forces the fallback path (error → profile step), so the
  // test asserts pre-existing-profile behavior without the default mock
  // identity's has_profile_event:true triggering the skip.
  await installMockBridge(
    page,
    {
      pendingCommunityDeepLinks: [PENDING_ADD_COMMUNITY_LINK],
      profileReadError: "no-kind-0",
    },
    { skipCommunitySeed: true },
  );
  await page.goto("/");

  await expect(page.getByTestId("community-onboarding-flow")).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Build your profile" }),
  ).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        (key) => window.localStorage.getItem(key),
        TRANSACTION_STORAGE_KEY,
      ),
    )
    .toContain('"source":"add-community"');
  await expect
    .poll(() =>
      page.evaluate(
        (key) => window.localStorage.getItem(key),
        TRANSACTION_STORAGE_KEY,
      ),
    )
    .toContain('"communityName":"Acme Team"');
});

test("add-community deep link skips profile step when identity has an existing kind:0 profile", async ({
  page,
}) => {
  // The default mock identity (deadbeef...) is pre-seeded with
  // has_profile_event:true. The skip should fire on connecting → clear the
  // transaction entirely, never showing the profile step.
  await installMockBridge(
    page,
    { pendingCommunityDeepLinks: [PENDING_ADD_COMMUNITY_LINK] },
    { skipCommunitySeed: true },
  );
  await page.goto("/");

  // Onboarding flow must disappear — the skip cleared the transaction.
  await expect(page.getByTestId("community-onboarding-flow")).toHaveCount(0);
  // handleCommunityOnboardingConnect already added the community when the
  // transaction reached "connecting", so the app lands in the full UI.
  await expect(page.getByTestId("sidebar-profile-avatar-button")).toBeVisible();
});

test("add-community deep link opens one editable prefill and acknowledges the queue", async ({
  page,
}) => {
  await installMockBridge(
    page,
    { pendingCommunityDeepLinks: [PENDING_ADD_COMMUNITY_LINK] },
    { seedPreviewFeatures: true },
  );
  await page.goto("/");

  await expect(
    page.getByRole("heading", { name: "Add Community" }),
  ).toBeVisible();
  const relayInput = page.locator("#ws-relay-url");
  const nameInput = page.locator("#ws-name");
  await expect(relayInput).toHaveValue(PENDING_ADD_COMMUNITY_LINK.relayUrl);
  await expect(nameInput).toHaveValue(PENDING_ADD_COMMUNITY_LINK.name);

  await nameInput.fill("Edited Team");
  await expect(nameInput).toHaveValue("Edited Team");

  await page.getByRole("button", { name: "Cancel" }).click();
  await expect(
    page.getByRole("heading", { name: "Add Community" }),
  ).toHaveCount(0);

  await page.getByTestId("sidebar-profile-avatar-button").click();
  await page.getByTestId("community-switcher").click();
  await page.getByRole("menuitem", { name: "Add Community" }).click();
  await expect(relayInput).toHaveValue("");
  await expect(nameInput).toHaveValue("");

  const acknowledgements = await page.evaluate(() =>
    (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).filter(
      (entry) => entry.command === "acknowledge_pending_community_deep_link",
    ),
  );
  expect(acknowledgements).toEqual([
    {
      command: "acknowledge_pending_community_deep_link",
      payload: { id: PENDING_ADD_COMMUNITY_LINK.id },
    },
  ]);
});

test("queued add-community links open and acknowledge one at a time", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      pendingCommunityDeepLinks: [
        PENDING_ADD_COMMUNITY_LINK,
        SECOND_PENDING_ADD_COMMUNITY_LINK,
      ],
    },
    { seedPreviewFeatures: true },
  );
  await page.goto("/");

  const relayInput = page.locator("#ws-relay-url");
  const nameInput = page.locator("#ws-name");
  await expect(relayInput).toHaveValue(PENDING_ADD_COMMUNITY_LINK.relayUrl);
  await expect(nameInput).toHaveValue(PENDING_ADD_COMMUNITY_LINK.name);

  await expect
    .poll(() =>
      page.evaluate(() =>
        (window.__BUZZ_E2E_COMMAND_LOG__ ?? [])
          .filter(
            (entry) =>
              entry.command === "acknowledge_pending_community_deep_link",
          )
          .map((entry) => entry.payload),
      ),
    )
    .toEqual([{ id: PENDING_ADD_COMMUNITY_LINK.id }]);

  await page.getByRole("button", { name: "Cancel" }).click();

  await expect(relayInput).toHaveValue(
    SECOND_PENDING_ADD_COMMUNITY_LINK.relayUrl,
  );
  await expect(nameInput).toHaveValue(SECOND_PENDING_ADD_COMMUNITY_LINK.name);
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window.__BUZZ_E2E_COMMAND_LOG__ ?? [])
          .filter(
            (entry) =>
              entry.command === "acknowledge_pending_community_deep_link",
          )
          .map((entry) => entry.payload),
      ),
    )
    .toEqual([
      { id: PENDING_ADD_COMMUNITY_LINK.id },
      { id: SECOND_PENDING_ADD_COMMUNITY_LINK.id },
    ]);
});

test("Welcome failure retries once before allowing starter channel setup to be skipped", async ({
  page,
}) => {
  const welcomeError = "Channel creation is not permitted.";
  await seedActiveIdentity(page, TEST_IDENTITIES.tyler);
  await page.addInitScript(
    ({ pubkey, relayUrl, storageKey }) => {
      window.localStorage.setItem(
        `buzz-machine-onboarding-complete.v2:${pubkey}`,
        "true",
      );
      const timestamp = new Date().toISOString();
      window.localStorage.setItem(
        storageKey,
        JSON.stringify({
          id: "txn-welcome-failure-1",
          source: "deep-link-join",
          stage: "team-intro",
          relayUrl,
          communityName: "hive",
          communityId: "e2e-default-community",
          createdAt: timestamp,
          updatedAt: timestamp,
        }),
      );
    },
    {
      pubkey: WELCOME_FAILURE_PUBKEY,
      relayUrl: COMMUNITY_RELAY_URL,
      storageKey: TRANSACTION_STORAGE_KEY,
    },
  );
  await installMockBridge(
    page,
    { ensureStarterChannelsErrors: [welcomeError, welcomeError, welcomeError] },
    { relayWsUrl: COMMUNITY_RELAY_URL, skipOnboardingSeed: true },
  );
  await page.goto("/");

  for (const name of ["fizz", "honey", "bumble"]) {
    const character = page.getByTestId(`starter-persona-${name}`);
    await expect(character).toBeVisible();
    await expect(character).toHaveAttribute(
      "src",
      `/onboarding/starter-team/${name}.png`,
    );
  }

  const enterButton = page.getByRole("button", { name: "Take me to Buzz" });
  await enterButton.click();

  await expect(page.getByText(`${welcomeError} Try again.`)).toBeVisible();
  await expect(enterButton).toBeEnabled();
  const backButton = page.getByRole("button", { name: "Back" });
  await expect(backButton).toBeVisible();
  await backButton.click();

  await expect(
    page.getByRole("heading", { name: "Build your profile" }),
  ).toBeVisible();
  await page.getByLabel("Community username").fill("Tyler");
  await page.getByTestId("community-profile-next").click();

  await enterButton.click();
  await expect(page.getByText(`${welcomeError} Try again.`)).toBeVisible();
  await expect(enterButton).toBeEnabled();

  await enterButton.click();

  const skipButton = page.getByRole("button", { name: "Skip for now" });
  await expect(page.getByText(welcomeError, { exact: true })).toBeVisible();
  await expect(skipButton).toBeEnabled();
  await expect(page.getByRole("button", { name: "Back" })).toBeVisible();

  const starterChannelAttempts = await page.evaluate(
    () =>
      window.__BUZZ_E2E_COMMANDS__?.filter(
        (command) => command === "ensure_starter_channels",
      ).length ?? 0,
  );
  expect(starterChannelAttempts).toBe(3);

  await skipButton.click();

  await expect(page.getByTestId("community-onboarding-flow")).toHaveCount(0);
  expect(
    await page.evaluate(
      () =>
        window.__BUZZ_E2E_COMMANDS__?.filter(
          (command) => command === "ensure_starter_channels",
        ).length ?? 0,
    ),
  ).toBe(3);
  await expect
    .poll(() =>
      page.evaluate(
        (transaction) => window.localStorage.getItem(transaction),
        TRANSACTION_STORAGE_KEY,
      ),
    )
    .toBeNull();
});

test("persisted deep-link invite hands off to Joining after machine onboarding", async ({
  page,
}) => {
  // Deterministic claim failure (no real relay behind the mock bridge): the
  // spec asserts the handoff reaches the "Joining …" claiming screen, not
  // that the claim itself succeeds.
  await page.route("**/api/invites/claim", (route) => route.abort());
  await page.addInitScript(
    ({ pubkey, storageKey }) => {
      window.localStorage.setItem(
        `buzz-machine-onboarding-complete.v2:${pubkey}`,
        "true",
      );
      const timestamp = new Date().toISOString();
      window.localStorage.setItem(
        storageKey,
        JSON.stringify({
          id: "txn-deep-link-1",
          source: "deep-link-join",
          stage: "claiming",
          relayUrl: "wss://hive.example.com",
          inviteCode: "abc.def",
          communityName: "hive",
          createdAt: timestamp,
          updatedAt: timestamp,
        }),
      );
    },
    { pubkey: DEFAULT_MOCK_PUBKEY, storageKey: TRANSACTION_STORAGE_KEY },
  );
  await installMockBridge(page, undefined, {
    skipCommunitySeed: true,
    skipOnboardingSeed: true,
  });
  await page.goto("/");

  // Machine onboarding is complete, so the transaction owns the screen.
  await expect(page.getByTestId("community-onboarding-flow")).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Joining hive" }),
  ).toBeVisible();
  await expect(page.getByTestId("pending-invite-gate")).toHaveCount(0);

  // The claim was attempted and its failure surfaced with a Retry.
  await expect(page.getByRole("button", { name: "Retry" })).toBeVisible();
});
