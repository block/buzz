import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

// Pending-invite acknowledgment for deep links that arrive before machine
// onboarding completes. The Rust side queues buzz://join / buzz://connect
// links; the frontend drains them into a persisted community-onboarding
// transaction the moment the app boots — even while the identity steps are
// still on screen — and overlays the PendingInviteGate so the link visibly
// reacts instead of silently waiting behind "Welcome to Buzz".

const DEFAULT_MOCK_PUBKEY = "deadbeef".repeat(8);
const TRANSACTION_STORAGE_KEY = "buzz-community-onboarding-transaction.v1";

test("join deep link during machine onboarding shows the pending-invite gate", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      pendingCommunityDeepLinks: [
        {
          id: "dl-join-1",
          kind: "join",
          relayUrl: "wss://hive.example.com",
          code: "abc.def",
        },
      ],
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");

  // The invite is acknowledged on screen while machine onboarding is pending.
  const gate = page.getByTestId("pending-invite-gate");
  await expect(gate).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Opening your invite" }),
  ).toBeVisible();
  await expect(gate).toContainText("You've been invited to join hive.");

  // The drain persisted the invite, so it survives a relaunch mid-onboarding.
  await expect
    .poll(() =>
      page.evaluate(
        (key) => window.localStorage.getItem(key),
        TRANSACTION_STORAGE_KEY,
      ),
    )
    .toContain("deep-link-join");

  // Continue returns to the identity steps; the claim runs after setup.
  await page.getByTestId("pending-invite-continue").click();
  await expect(gate).toHaveCount(0);
  await expect(page.getByTestId("machine-onboarding-gate")).toBeVisible();
  await expect(page.getByRole("button", { name: "Get started" })).toBeVisible();
});

test("connect deep link during machine onboarding shows the community-link gate", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      pendingCommunityDeepLinks: [
        {
          id: "dl-connect-1",
          kind: "connect",
          relayUrl: "wss://hive.example.com",
        },
      ],
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");

  const gate = page.getByTestId("pending-invite-gate");
  await expect(gate).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Opening community link" }),
  ).toBeVisible();
  await expect(gate).toContainText("You're connecting to hive.");
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
