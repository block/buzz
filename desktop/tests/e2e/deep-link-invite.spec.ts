import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

// Invite deep links that arrive before machine onboarding completes. The
// Rust side queues buzz://join links; the frontend drains them into a
// persisted community-onboarding transaction the moment the app boots and
// overlays the PendingInviteGate: a loading state that confirms the invite
// against its relay right away, then auto-dismisses back into the identity
// steps. The remaining join (add community, profile) resumes after setup.

const DEFAULT_MOCK_PUBKEY = "deadbeef".repeat(8);
const TRANSACTION_STORAGE_KEY = "buzz-community-onboarding-transaction.v1";

const PENDING_JOIN_LINK = {
  id: "dl-join-1",
  kind: "join" as const,
  relayUrl: "wss://hive.example.com",
  code: "abc.def",
};

test("join deep link shows the invite loader and auto-advances into setup", async ({
  page,
}) => {
  await page.route("**/api/invites/claim", async (route) => {
    await new Promise((resolve) => setTimeout(resolve, 800));
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        status: "joined",
        community_id: "demo-community",
        host: "hive.example.com",
        role: "member",
      }),
    });
  });
  await installMockBridge(
    page,
    { pendingCommunityDeepLinks: [PENDING_JOIN_LINK] },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");

  // The loader appears while the claim is in flight.
  const gate = page.getByTestId("pending-invite-gate");
  await expect(gate).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Opening your invite" }),
  ).toBeVisible();

  // On success it auto-dismisses into the identity steps — no click needed.
  await expect(gate).toHaveCount(0);
  await expect(page.getByTestId("machine-onboarding-gate")).toBeVisible();
  await expect(page.getByRole("button", { name: "Get started" })).toBeVisible();

  // The confirmed invite is persisted past the claim, so the join resumes
  // after setup (and survives a relaunch in between).
  await expect
    .poll(() =>
      page.evaluate(
        (key) => window.localStorage.getItem(key),
        TRANSACTION_STORAGE_KEY,
      ),
    )
    .toContain('"stage":"connecting"');
});

test("failed invite confirmation offers Retry and Cancel returns to setup", async ({
  page,
}) => {
  await page.route("**/api/invites/claim", (route) =>
    route.fulfill({
      status: 410,
      contentType: "application/json",
      body: JSON.stringify({ error: "invite_expired" }),
    }),
  );
  await installMockBridge(
    page,
    { pendingCommunityDeepLinks: [PENDING_JOIN_LINK] },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");

  const gate = page.getByTestId("pending-invite-gate");
  await expect(gate).toBeVisible();
  await expect(gate).toContainText("invite_expired");
  await expect(page.getByTestId("pending-invite-retry")).toBeVisible();

  // Cancel abandons the invite and drops back to the identity steps.
  await page.getByTestId("pending-invite-cancel").click();
  await expect(gate).toHaveCount(0);
  await expect(page.getByTestId("machine-onboarding-gate")).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        (key) => window.localStorage.getItem(key),
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
