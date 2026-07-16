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

test("Welcome failure can be skipped without abandoning community onboarding", async ({
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
    { ensureStarterChannelsErrors: [welcomeError, welcomeError] },
    { relayWsUrl: COMMUNITY_RELAY_URL, skipOnboardingSeed: true },
  );
  await page.goto("/");

  await page.getByRole("button", { name: "Enter hive" }).click();

  await expect(page.getByText(welcomeError)).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Preparing Welcome…" }),
  ).toBeEnabled();
  const skip = page.getByRole("button", { name: "Skip for now" });
  await expect(skip).toBeVisible();
  await skip.click();

  const completionKey = `buzz-community-onboarding-complete.v1:${encodeURIComponent(COMMUNITY_RELAY_URL)}:${WELCOME_FAILURE_PUBKEY}`;
  await expect
    .poll(() =>
      page.evaluate(
        ({ completion, transaction }) => ({
          completion: window.localStorage.getItem(completion),
          transaction: window.localStorage.getItem(transaction),
        }),
        { completion: completionKey, transaction: TRANSACTION_STORAGE_KEY },
      ),
    )
    .toEqual({ completion: "true", transaction: null });
  await expect(page.getByTestId("community-onboarding-flow")).toHaveCount(0);
  await expect(page.getByTestId("app-sidebar")).toBeVisible();
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
