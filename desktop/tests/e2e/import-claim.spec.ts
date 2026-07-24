import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/import-claim";

type SignedEvent = { kind: number; tags: string[][] };

/**
 * Deliver a `buzz://import-claim` deep link the way the native Tauri plugin
 * does — via the mockIPC event system (`plugin:event|emit`), which fans out to
 * the app's `listen("deep-link-import-claim", …)` subscriber. Re-emits under
 * `toPass` so the first emit can't race the listener's async registration.
 */
async function emitImportClaim(
  page: import("@playwright/test").Page,
  payload: Record<string, string>,
) {
  const dialog = page.getByRole("dialog");
  await expect(async () => {
    await page.evaluate(
      (p) =>
        (
          window as unknown as {
            __TAURI_INTERNALS__: {
              invoke: (cmd: string, args: unknown) => Promise<unknown>;
            };
          }
        ).__TAURI_INTERNALS__.invoke("plugin:event|emit", {
          event: "deep-link-import-claim",
          payload: p,
        }),
      payload,
    );
    await expect(dialog).toBeVisible({ timeout: 500 });
  }).toPass({ timeout: 5_000 });
}

test.beforeEach(async ({ page }) => {
  // Seed the signed-event recorder before the bridge installs its init scripts.
  await page.addInitScript(() => {
    (
      window as unknown as { __BUZZ_E2E_SIGNED_EVENTS__: SignedEvent[] }
    ).__BUZZ_E2E_SIGNED_EVENTS__ = [];
  });
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page
    .getByTestId("app-sidebar")
    .waitFor({ state: "visible", timeout: 15_000 });
});

test("join-slack callback: connects the target relay before self-claim", async ({
  page,
}) => {
  const now = new Date().toISOString();
  await page.evaluate(
    ({ createdAt }) => {
      localStorage.setItem(
        "buzz-community-onboarding-transaction.v1",
        JSON.stringify({
          id: "slack-join-e2e",
          source: "deep-link-join-slack",
          stage: "slack-auth",
          relayUrl: "ws://localhost:3000",
          communityName: "E2E Test",
          slackService: "http://mock.local",
          createdAt,
          updatedAt: createdAt,
        }),
      );
    },
    { createdAt: now },
  );
  await page.reload({ waitUntil: "domcontentloaded" });

  await emitImportClaim(page, {
    subject: "slack:U060",
    via: "oidc",
    relayUrl: "ws://localhost:3000",
    service: "http://mock.local",
  });

  const dialog = page.getByRole("dialog");
  await expect(dialog.getByText("Link your imported history")).toBeVisible();
  await expect(dialog.getByText("slack:U060")).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/oidc-confirm.png` });

  await dialog.getByRole("button", { name: "Link my history" }).click();
  await expect(dialog).toBeHidden({ timeout: 10_000 });

  // The target community becomes active before the deferred subject
  // self-claim is signed and published.
  let signed: SignedEvent[] = [];
  await expect(async () => {
    signed = await page.evaluate(
      () =>
        (window as unknown as { __BUZZ_E2E_SIGNED_EVENTS__: SignedEvent[] })
          .__BUZZ_E2E_SIGNED_EVENTS__,
    );
    expect(signed.some((event) => event.kind === 30624)).toBe(true);
  }).toPass({ timeout: 10_000 });
  const claim = signed.find((e) => e.kind === 30624);
  expect(claim, "a kind-30624 self-claim should have been signed").toBeTruthy();
  expect(claim?.tags).toContainEqual(["d", "slack:U060"]);
  expect(claim?.tags).toContainEqual(["import", "slack"]);
});

test("email import-claim: POSTs token + own pubkey to the service, then done", async ({
  page,
}) => {
  let posted: { token?: string; pubkey?: string } | null = null;
  await page.route("**/email/complete", async (route) => {
    posted = JSON.parse(route.request().postData() ?? "{}");
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        subject: "slack:U081",
        attestation_event_id: "mock-attestation",
      }),
    });
  });

  await emitImportClaim(page, {
    subject: "slack:U081",
    token: "v1.mock.token",
    service: "http://mock.local",
  });

  const dialog = page.getByRole("dialog");
  await expect(dialog.getByText("slack:U081")).toBeVisible();
  await dialog.getByRole("button", { name: "Link my history" }).click();
  await expect(dialog.getByText(/now show under your account/i)).toBeVisible({
    timeout: 10_000,
  });

  // The app redeemed the magic-link token with the service, sending the token
  // and its OWN 64-hex pubkey (never a private key).
  expect(posted, "the app should POST to /email/complete").toBeTruthy();
  expect(posted?.token).toBe("v1.mock.token");
  expect(typeof posted?.pubkey).toBe("string");
  expect((posted?.pubkey ?? "").length).toBe(64);
});
