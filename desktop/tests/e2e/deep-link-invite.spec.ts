import { expect, test, type Page } from "@playwright/test";
import { hexToBytes } from "@noble/hashes/utils.js";
import { nsecEncode } from "nostr-tools/nip19";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { seedActiveIdentity } from "../helpers/onboarding";

// Community deep links that arrive before machine onboarding complete are
// drained from Rust into a persisted transaction and acknowledged immediately.
// Invite claiming waits until setup finishes and the final identity is known.

const DEFAULT_MOCK_PUBKEY = "deadbeef".repeat(8);
const COMMUNITY_ONBOARDING_PUBKEY = TEST_IDENTITIES.tyler.pubkey;
const TRANSACTION_STORAGE_KEY = "buzz-community-onboarding-transaction.v1";
const COMMUNITY_RELAY_URL = "wss://hive.example.com";
const IDENTITY_HANDOFF_CODE = `v3.${"a".repeat(64)}`;
const JOIN_POLICY = {
  terms_markdown: "# Community terms",
  privacy_markdown: "# Community privacy",
  age_attestation_required: true,
  version: "policy-v1",
};

function pendingIdentityHandoff(id: string) {
  return {
    id,
    kind: "join" as const,
    relayUrl: COMMUNITY_RELAY_URL,
    code: IDENTITY_HANDOFF_CODE,
  };
}

async function expectNoHandoffCredentialInStorage(page: Page) {
  const persisted = await page.evaluate(
    (key) => window.localStorage.getItem(key),
    TRANSACTION_STORAGE_KEY,
  );
  expect(persisted ?? "").not.toContain(IDENTITY_HANDOFF_CODE);
  expect(persisted ?? "").not.toContain("policy-receipt");
}

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
    page.getByRole("heading", { name: "Join an existing community" }),
  ).toBeVisible();
  const communityInput = page.getByLabel("Community URL or invite link");
  await expect(communityInput).toHaveValue(PENDING_ADD_COMMUNITY_LINK.relayUrl);
  await expect(page.getByLabel("Name")).toHaveCount(0);

  await page.getByRole("button", { name: "Close" }).click();
  await expect(
    page.getByRole("heading", { name: "Join an existing community" }),
  ).toHaveCount(0);

  await page.getByTestId("sidebar-profile-avatar-button").click();
  await page.getByTestId("community-switcher").click();
  await page.getByRole("menuitem", { name: "Add a community" }).click();
  await page.getByTestId("add-community-join").click();
  await expect(communityInput).toHaveValue("");

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

  const communityInput = page.getByLabel("Community URL or invite link");
  await expect(communityInput).toHaveValue(PENDING_ADD_COMMUNITY_LINK.relayUrl);

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

  await page.getByRole("button", { name: "Close" }).click();

  await expect(communityInput).toHaveValue(
    SECOND_PENDING_ADD_COMMUNITY_LINK.relayUrl,
  );
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

test("deleted public starter channels do not strand community onboarding", async ({
  page,
}) => {
  const starterError =
    "starter channels created but metadata not yet available";
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
          id: "txn-deleted-starters-1",
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
      pubkey: COMMUNITY_ONBOARDING_PUBKEY,
      relayUrl: COMMUNITY_RELAY_URL,
      storageKey: TRANSACTION_STORAGE_KEY,
    },
  );
  await installMockBridge(
    page,
    { ensureStarterChannelsErrors: [starterError] },
    { relayWsUrl: COMMUNITY_RELAY_URL, skipOnboardingSeed: true },
  );
  await page.goto("/");

  await page.getByRole("button", { name: "Take me to Buzz" }).click();

  await expect(page.getByTestId("community-onboarding-flow")).toHaveCount(0);
  await expect(page).toHaveURL(/#\/channels\/[^/]+$/);
  await expect(page.getByTestId("chat-title")).toContainText("Welcome");
  await expect(page.getByText(starterError)).toHaveCount(0);
  expect(
    await page.evaluate(
      () =>
        window.__BUZZ_E2E_COMMANDS__?.filter(
          (command) => command === "ensure_starter_channels",
        ).length ?? 0,
    ),
  ).toBe(1);
});

test("required Welcome creation failure keeps community onboarding open", async ({
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
      pubkey: COMMUNITY_ONBOARDING_PUBKEY,
      relayUrl: COMMUNITY_RELAY_URL,
      storageKey: TRANSACTION_STORAGE_KEY,
    },
  );
  await installMockBridge(
    page,
    { createChannelErrors: [welcomeError] },
    { relayWsUrl: COMMUNITY_RELAY_URL, skipOnboardingSeed: true },
  );
  await page.goto("/");

  await page.getByRole("button", { name: "Take me to Buzz" }).click();

  await expect(page.getByTestId("community-onboarding-flow")).toBeVisible();
  await expect(page.getByText(`${welcomeError} Try again.`)).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Take me to Buzz" }),
  ).toBeEnabled();
  await expect(page.getByTestId("chat-title")).toHaveCount(0);
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

test("bound identity accepts the current policy before claim", async ({
  page,
}) => {
  const acceptBodies: Array<Record<string, unknown>> = [];
  const claimBodies: Array<Record<string, unknown>> = [];
  let acceptAttempts = 0;
  await page.route("**/api/invites/accept-policy", async (route) => {
    acceptAttempts++;
    acceptBodies.push(
      route.request().postDataJSON() as Record<string, unknown>,
    );
    await route.fulfill({
      contentType: "application/json",
      status: acceptAttempts === 1 ? 503 : 200,
      body: JSON.stringify(
        acceptAttempts === 1 ? {} : { receipt: "fresh-policy-receipt" },
      ),
    });
  });
  await page.route("**/api/invites/claim", async (route) => {
    claimBodies.push(route.request().postDataJSON() as Record<string, unknown>);
    await route.fulfill({
      contentType: "application/json",
      status: 200,
      body: JSON.stringify({
        status: "already_member",
        community_id: "hive-community",
        host: "hive.example.com",
        role: "member",
      }),
    });
  });
  await installMockBridge(
    page,
    {
      joinPolicy: JOIN_POLICY,
      pendingCommunityDeepLinks: [pendingIdentityHandoff("dl-v3-policy")],
    },
    { relayWsUrl: COMMUNITY_RELAY_URL },
  );
  await page.goto("/");

  await expect(
    page.getByRole("heading", { name: "Review community requirements" }),
  ).toBeVisible();
  expect(claimBodies).toHaveLength(0);
  await page.getByLabel("I am 18 years of age or older.").click();
  await page.getByLabel(/I agree to the Buzz/).click();
  await page.getByRole("button", { name: "Accept and continue" }).click();

  await expect(page.getByTestId("identity-handoff-policy-error")).toContainText(
    "couldn’t confirm",
  );
  expect(claimBodies).toHaveLength(0);
  await page.getByRole("button", { name: "Accept and continue" }).click();

  await expect.poll(() => claimBodies.length).toBe(1);
  expect(acceptBodies).toEqual([
    {
      code: IDENTITY_HANDOFF_CODE,
      policy_version: "policy-v1",
      age_confirmed: true,
    },
    {
      code: IDENTITY_HANDOFF_CODE,
      policy_version: "policy-v1",
      age_confirmed: true,
    },
  ]);
  expect(claimBodies[0]).toEqual({
    code: IDENTITY_HANDOFF_CODE,
    policy_receipt: "fresh-policy-receipt",
    protocol: "identity-handoff-v3",
  });
  await expectNoHandoffCredentialInStorage(page);
});

test("a policy version change resets consent before acceptance", async ({
  page,
}) => {
  const acceptBodies: Array<Record<string, unknown>> = [];
  const claimBodies: Array<Record<string, unknown>> = [];
  await page.route("**/api/invites/accept-policy", async (route) => {
    acceptBodies.push(
      route.request().postDataJSON() as Record<string, unknown>,
    );
    await route.fulfill({
      contentType: "application/json",
      status: 200,
      body: JSON.stringify({ receipt: "policy-v2-receipt" }),
    });
  });
  await page.route("**/api/invites/claim", async (route) => {
    claimBodies.push(route.request().postDataJSON() as Record<string, unknown>);
    await route.fulfill({
      contentType: "application/json",
      status: 200,
      body: JSON.stringify({
        status: "already_member",
        community_id: "hive-community",
        host: "hive.example.com",
        role: "member",
      }),
    });
  });
  await installMockBridge(
    page,
    {
      joinPolicy: JOIN_POLICY,
      pendingCommunityDeepLinks: [
        pendingIdentityHandoff("dl-v3-policy-version"),
      ],
    },
    { relayWsUrl: COMMUNITY_RELAY_URL },
  );
  await page.goto("/");

  const ageCheckbox = page.getByLabel("I am 18 years of age or older.");
  const agreementCheckbox = page.getByLabel(/I agree to the Buzz/);
  await ageCheckbox.click();
  await agreementCheckbox.click();
  await page.evaluate(() => {
    const testWindow = window as Window & {
      __BUZZ_E2E__?: {
        mock?: {
          joinPolicy?: {
            terms_markdown?: string;
            privacy_markdown?: string;
            age_attestation_required: boolean;
            version: string;
          } | null;
        };
      };
    };
    if (testWindow.__BUZZ_E2E__?.mock) {
      testWindow.__BUZZ_E2E__.mock.joinPolicy = {
        terms_markdown: "# Updated terms",
        privacy_markdown: "# Updated privacy",
        age_attestation_required: true,
        version: "policy-v2",
      };
    }
  });
  await page.getByRole("button", { name: "Accept and continue" }).click();

  await expect(page.getByTestId("identity-handoff-policy-error")).toContainText(
    "changed",
  );
  await expect(ageCheckbox).not.toBeChecked();
  await expect(agreementCheckbox).not.toBeChecked();
  expect(acceptBodies).toHaveLength(0);
  expect(claimBodies).toHaveLength(0);

  await ageCheckbox.click();
  await agreementCheckbox.click();
  await page.getByRole("button", { name: "Accept and continue" }).click();
  await expect.poll(() => claimBodies.length).toBe(1);
  expect(acceptBodies[0]).toEqual({
    code: IDENTITY_HANDOFF_CODE,
    policy_version: "policy-v2",
    age_confirmed: true,
  });
});

test("policy discovery failure stays retryable and never claims", async ({
  page,
}) => {
  let claimCalls = 0;
  await page.route("**/api/invites/claim", async (route) => {
    claimCalls++;
    await route.abort();
  });
  await installMockBridge(
    page,
    {
      joinPolicy: null,
      joinPolicyErrors: ["credential-shaped relay detail", null],
      pendingCommunityDeepLinks: [pendingIdentityHandoff("dl-v3-policy-retry")],
    },
    { relayWsUrl: COMMUNITY_RELAY_URL },
  );
  await page.goto("/");

  const policyError = page.getByTestId("identity-handoff-policy-discovery");
  await expect(policyError).toContainText("couldn’t check");
  await expect(policyError).not.toContainText("credential-shaped");
  expect(claimCalls).toBe(0);

  await page.getByRole("button", { name: "Retry" }).click();
  await expect.poll(() => claimCalls).toBe(1);
});

test("bound identity mismatch requires backup, imports the saved key, and retries the same in-memory invite", async ({
  page,
}) => {
  const claimBodies: Array<Record<string, unknown>> = [];
  await page.route("**/api/invites/accept-policy", async (route) => {
    await route.fulfill({
      contentType: "application/json",
      status: 200,
      body: JSON.stringify({ receipt: "recovery-policy-receipt" }),
    });
  });
  await page.route("**/api/invites/claim", async (route) => {
    const body = route.request().postDataJSON() as Record<string, unknown>;
    claimBodies.push(body);
    await route.fulfill({
      contentType: "application/json",
      status: claimBodies.length === 1 ? 409 : 200,
      body: JSON.stringify(
        claimBodies.length === 1
          ? { error: "invite_identity_mismatch" }
          : {
              status: "joined",
              community_id: "hive-community",
              host: "hive.example.com",
              role: "member",
            },
      ),
    });
  });
  await installMockBridge(
    page,
    {
      joinPolicy: JOIN_POLICY,
      pendingCommunityDeepLinks: [pendingIdentityHandoff("dl-v3-mismatch")],
    },
    { relayWsUrl: COMMUNITY_RELAY_URL },
  );
  await page.goto("/");

  await page.getByLabel("I am 18 years of age or older.").click();
  await page.getByLabel(/I agree to the Buzz/).click();
  await page.getByRole("button", { name: "Accept and continue" }).click();

  const mismatchHeading = page.getByRole("heading", {
    name: "This invite belongs to your saved identity",
  });
  await expect(mismatchHeading).toBeVisible();
  await expect(mismatchHeading).toBeFocused();
  await expect(page.getByTestId("identity-handoff-mismatch")).not.toContainText(
    TEST_IDENTITIES.alice.pubkey,
  );
  await expectNoHandoffCredentialInStorage(page);

  const backupConfirm = page.getByTestId("identity-handoff-backup-confirm");
  const continueImport = page.getByTestId("identity-handoff-continue-import");
  await expect(backupConfirm).toBeDisabled();
  await expect(continueImport).toBeDisabled();

  const reveal = page.getByRole("button", { name: "Reveal private key" });
  await reveal.focus();
  await page.keyboard.press("Enter");
  await backupConfirm.focus();
  await page.keyboard.press("Space");
  await continueImport.focus();
  await page.keyboard.press("Enter");

  const keyInput = page.getByTestId("nostr-import-nsec-input");
  await expect(keyInput).toBeFocused();
  await keyInput.fill("not-a-private-key");
  await expect(page.getByTestId("nostr-import-feedback")).toContainText(
    "Waiting for a valid nsec1 key",
  );
  expect(claimBodies).toHaveLength(1);
  await expectNoHandoffCredentialInStorage(page);

  const savedNsec = nsecEncode(hexToBytes(TEST_IDENTITIES.alice.privateKey));
  await keyInput.fill(savedNsec);
  await keyInput.press("Enter");

  await expect.poll(() => claimBodies.length).toBe(2);
  expect(claimBodies).toEqual([
    {
      code: IDENTITY_HANDOFF_CODE,
      policy_receipt: "recovery-policy-receipt",
      protocol: "identity-handoff-v3",
    },
    {
      code: IDENTITY_HANDOFF_CODE,
      policy_receipt: "recovery-policy-receipt",
      protocol: "identity-handoff-v3",
    },
  ]);
  await expect(page.getByTestId("identity-handoff-mismatch")).toHaveCount(0);
  await expectNoHandoffCredentialInStorage(page);
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          window.__BUZZ_E2E_COMMANDS__?.filter(
            (command) => command === "import_identity",
          ).length ?? 0,
      ),
    )
    .toBe(1);
});

test("matching current identity claims a bound invite without import recovery", async ({
  page,
}) => {
  const claimBodies: Array<Record<string, unknown>> = [];
  await page.route("**/api/invites/claim", async (route) => {
    claimBodies.push(route.request().postDataJSON() as Record<string, unknown>);
    await route.fulfill({
      contentType: "application/json",
      status: 200,
      body: JSON.stringify({
        status: "already_member",
        community_id: "hive-community",
        host: "hive.example.com",
        role: "member",
      }),
    });
  });
  await installMockBridge(
    page,
    { pendingCommunityDeepLinks: [pendingIdentityHandoff("dl-v3-match")] },
    { relayWsUrl: COMMUNITY_RELAY_URL },
  );
  await page.goto("/");

  await expect.poll(() => claimBodies.length).toBe(1);
  expect(claimBodies[0]).toEqual({
    code: IDENTITY_HANDOFF_CODE,
    protocol: "identity-handoff-v3",
  });
  expect(
    await page.evaluate(
      () =>
        window.__BUZZ_E2E_COMMANDS__?.filter(
          (command) => command === "fetch_join_policy",
        ).length ?? 0,
    ),
  ).toBe(1);
  await expect(page.getByTestId("identity-handoff-mismatch")).toHaveCount(0);
  expect(
    await page.evaluate(
      () => window.__BUZZ_E2E_COMMANDS__?.includes("import_identity") ?? false,
    ),
  ).toBe(false);
});

for (const terminal of [
  "invite_expired",
  "invite_superseded",
  "invite_invalidated",
] as const) {
  test(`bound identity ${terminal} directs the user to mint a fresh link`, async ({
    page,
  }) => {
    await page.route("**/api/invites/claim", async (route) => {
      await route.fulfill({
        contentType: "application/json",
        status: terminal === "invite_superseded" ? 409 : 403,
        body: JSON.stringify({ error: terminal }),
      });
    });
    await installMockBridge(
      page,
      {
        pendingCommunityDeepLinks: [
          pendingIdentityHandoff(`dl-v3-${terminal}`),
        ],
      },
      { relayWsUrl: COMMUNITY_RELAY_URL },
    );
    await page.goto("/");

    const terminalSummary = page.getByTestId("identity-handoff-terminal");
    await expect(terminalSummary).toBeVisible();
    await expect(terminalSummary).toHaveAttribute("aria-live", "assertive");
    await expect(terminalSummary).toContainText("request a fresh link");
    await expectNoHandoffCredentialInStorage(page);
    await page.getByTestId("identity-handoff-back-dashboard").click();
    await expect
      .poll(() =>
        page.evaluate(
          (key) => window.localStorage.getItem(key),
          TRANSACTION_STORAGE_KEY,
        ),
      )
      .toBeNull();
  });
}

test("restart abandons persisted v3 metadata and requires a fresh dashboard link", async ({
  page,
}) => {
  await page.addInitScript(
    ({ storageKey, relayUrl }) => {
      const timestamp = new Date().toISOString();
      window.localStorage.setItem(
        storageKey,
        JSON.stringify({
          id: "txn-v3-restart",
          source: "deep-link-join",
          stage: "claiming",
          relayUrl,
          communityName: "hive",
          inviteProtocol: "identity-handoff-v3",
          createdAt: timestamp,
          updatedAt: timestamp,
        }),
      );
    },
    { storageKey: TRANSACTION_STORAGE_KEY, relayUrl: COMMUNITY_RELAY_URL },
  );
  await installMockBridge(page, undefined, {
    relayWsUrl: COMMUNITY_RELAY_URL,
  });
  await page.goto("/");

  await expect(page.getByTestId("identity-handoff-terminal")).toContainText(
    "Buzz restarted before the handoff finished",
  );
  await expectNoHandoffCredentialInStorage(page);
});

test("backup retrieval failure blocks replacement until a successful retry", async ({
  page,
}) => {
  await page.route("**/api/invites/claim", async (route) => {
    await route.fulfill({
      contentType: "application/json",
      status: 409,
      body: JSON.stringify({ error: "invite_identity_mismatch" }),
    });
  });
  await installMockBridge(
    page,
    {
      nsecErrors: ["keyring unavailable", null],
      pendingCommunityDeepLinks: [pendingIdentityHandoff("dl-v3-backup")],
    },
    { relayWsUrl: COMMUNITY_RELAY_URL },
  );
  await page.goto("/");

  await expect(page.getByTestId("identity-handoff-backup-error")).toContainText(
    "Nothing was replaced",
  );
  await expect(
    page.getByTestId("identity-handoff-backup-error"),
  ).not.toContainText("keyring unavailable");
  await expect(
    page.getByTestId("identity-handoff-backup-confirm"),
  ).toBeDisabled();
  await expect(
    page.getByTestId("identity-handoff-continue-import"),
  ).toBeDisabled();

  await page.getByTestId("identity-handoff-backup-retry").click();
  await expect(
    page.getByRole("button", { name: "Reveal private key" }),
  ).toBeVisible();
  await expect(
    page.getByTestId("identity-handoff-backup-confirm"),
  ).toBeDisabled();
});
