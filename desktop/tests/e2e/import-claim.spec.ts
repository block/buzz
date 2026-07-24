import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

type SignedEvent = { kind: number; tags: string[][] };

/**
 * Deliver a `buzz://import-claim` deep link the way the native Tauri plugin
 * does — via the mockIPC event system (`plugin:event|emit`), which fans out to
 * the app's `listen("deep-link-import-claim", …)` subscriber.
 */
async function emitImportClaim(
  page: import("@playwright/test").Page,
  payload: Record<string, string>,
) {
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

test("join-slack callback: automatically connects before self-claim", async ({
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
  await expect(page.getByText("Join E2E Test with Slack")).toBeVisible();

  // The app must redeem the finalize code with a signed self-claim BEFORE the
  // attestation exists — capture what it POSTs to prove the pubkey is proven,
  // not server-trusted.
  let finalize: {
    code?: string;
    claim?: { kind?: number; tags?: string[][] };
  } = {};
  await page.route("**/oidc/finalize", async (route) => {
    finalize = JSON.parse(route.request().postData() ?? "{}");
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        subject: "slack:T060:U060",
        attestation_event_id: "mock-attestation",
      }),
    });
  });

  await emitImportClaim(page, {
    subject: "slack:T060:U060",
    via: "oidc",
    code: "oidc-code-1",
    relayUrl: "ws://localhost:3000",
    service: "http://mock.local",
  });

  await expect(page.getByRole("dialog")).toBeHidden();

  // The target community becomes active, then the app finalizes: it POSTs the
  // signed self-claim to /oidc/finalize and publishes the same kind-30624 event.
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
  expect(claim?.tags).toContainEqual(["d", "slack:T060:U060"]);
  expect(claim?.tags).toContainEqual(["import", "slack"]);

  // Finalize received the single-use code and the signed self-claim as proof of
  // possession — the pubkey being attested is the one that signed. The POST is
  // awaited inside the finalize helper, so poll until the route captured it.
  await expect(() => {
    expect(finalize.code).toBe("oidc-code-1");
    expect(finalize.claim?.kind).toBe(30624);
    expect(finalize.claim?.tags).toContainEqual(["d", "slack:T060:U060"]);
  }).toPass({ timeout: 10_000 });
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
  await expect(dialog).toBeVisible();
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
