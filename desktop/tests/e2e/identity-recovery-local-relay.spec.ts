import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

// A fresh desktop with no community relay configured falls back to the
// built-in `ws://localhost:3000` dev default and embeds it in the recovery
// QR. A phone scanning that code tries to reach *itself*. The recovery
// dialog must say so instead of rendering an unusable code.

test("recovery code pointing at a local-only relay explains the cause instead of showing a QR", async ({
  page,
}, testInfo) => {
  await installMockBridge(
    page,
    { identityLost: true, pairingRelayUrl: "ws://localhost:3000" },
    { skipOnboardingSeed: true },
  );
  await page.goto("/");

  await page.getByTestId("nostr-import-phone-link").click();
  const card = page.getByTestId("identity-recovery-pairing");
  await expect(card).toBeVisible();

  const notice = card.getByTestId("identity-recovery-local-relay");
  await expect(notice).toBeVisible();
  await expect(notice).toContainText("ws://localhost:3000");
  await expect(notice).toContainText("A phone scanning it can't get there");
  await expect(
    page.getByText("This desktop isn't connected to a community yet."),
  ).toBeVisible();

  // No unusable code, and nothing to copy.
  await expect(card.getByTestId("identity-recovery-qr")).toHaveCount(0);
  await expect(card.getByTestId("copy-identity-recovery-code")).toHaveCount(0);

  // The backend session against the unreachable relay was torn down.
  const commands = await page.evaluate(
    () =>
      (window as Window & { __BUZZ_E2E_COMMANDS__?: string[] })
        .__BUZZ_E2E_COMMANDS__ ?? [],
  );
  expect(commands).toContain("cancel_pairing");

  await page.waitForTimeout(1_000); // Let the onboarding entrance motion settle.
  await waitForAnimations(page);
  await page.screenshot({
    path: testInfo.outputPath("desktop-recovery-local-relay.png"),
  });

  // Developers pairing a simulator against a local relay keep a way through.
  await card.getByTestId("identity-recovery-local-relay-show-anyway").click();
  await expect(card.getByTestId("identity-recovery-qr")).toBeVisible();
  await expect(card.getByTestId("copy-identity-recovery-code")).toBeVisible();
  await expect(
    page.getByText("Scan this code with a signed-in Buzz phone."),
  ).toBeVisible();
});

test("recovery code pointing at a public relay still shows the QR", async ({
  page,
}) => {
  await installMockBridge(
    page,
    { identityLost: true, pairingRelayUrl: "wss://pairing.buzz.xyz" },
    { skipOnboardingSeed: true },
  );
  await page.goto("/");

  await page.getByTestId("nostr-import-phone-link").click();
  const card = page.getByTestId("identity-recovery-pairing");
  await expect(card.getByTestId("identity-recovery-qr")).toBeVisible();
  await expect(card.getByTestId("identity-recovery-local-relay")).toHaveCount(
    0,
  );
});
