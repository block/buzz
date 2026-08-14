import { hexToBytes } from "@noble/hashes/utils.js";
import { expect, test } from "@playwright/test";
import { nsecEncode } from "nostr-tools/nip19";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

test("normal first launch uses the already-persisted identity", async ({
  page,
}) => {
  await page.emulateMedia({ colorScheme: "dark" });
  await installMockBridge(page, undefined, {
    skipCommunitySeed: true,
    skipOnboardingSeed: true,
  });
  await page.goto("/");

  const gate = page.getByTestId("machine-onboarding-gate");
  await expect(gate).toBeVisible();
  await expect(gate).toHaveCSS("background-color", "rgb(215, 215, 46)");
  // Landing carries a subtle dot-grid pattern over the chartreuse fill.
  await expect(gate).toHaveCSS("background-image", /radial-gradient/);
  await expect(gate).toHaveCSS("color", "rgb(23, 23, 23)");
  await expect(
    page.getByRole("button", { name: "Create a new identity key" }),
  ).toHaveCSS("background-color", "rgb(23, 23, 23)");
  await page.getByRole("button", { name: "Create a new identity key" }).click();

  await expect(
    page.getByRole("heading", {
      name: "Your unique identity key has been created",
    }),
  ).toBeVisible();
  // Non-landing pages layer the dot grid over the chartreuse→light-blue gradient.
  await expect(gate).toHaveCSS(
    "background-image",
    /radial-gradient\(.*\), linear-gradient\(.*rgb\(215, 215, 46\).*rgb\(215, 231, 246\)\)/s,
  );
  await expect(gate).toHaveCSS("color", "rgb(23, 23, 23)");
  const commands = await page.evaluate(
    () =>
      (
        window as Window & {
          __BUZZ_E2E_COMMAND_PAYLOADS__?: Array<{ command: string }>;
        }
      ).__BUZZ_E2E_COMMAND_PAYLOADS__ ?? [],
  );
  expect(commands.some((entry) => entry.command === "get_identity")).toBe(true);
  expect(
    commands.some((entry) => entry.command === "persist_current_identity"),
  ).toBe(false);
});

test("lost boot opens onboarding gate directly on the key-import page", async ({
  page,
}, testInfo) => {
  await installMockBridge(
    page,
    { identityLost: true },
    { skipOnboardingSeed: true },
  );
  await page.goto("/");

  await expect(page.getByTestId("machine-onboarding-gate")).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Enter your private key" }),
  ).toBeVisible();
  await page.waitForTimeout(1_000);
  await page.screenshot({
    path: testInfo.outputPath("desktop-private-key-recovery.png"),
  });
});

test("lost boot keeps the pairing-code action stable while generating", async ({
  page,
}) => {
  await installMockBridge(
    page,
    { identityLost: true, pairingStartDelayMs: 2_500 },
    { skipOnboardingSeed: true },
  );
  await page.goto("/");

  await page.getByTestId("nostr-import-phone-link").click();
  const copyButton = page.getByTestId("copy-identity-recovery-code");
  await expect(copyButton).toBeVisible();
  await expect(copyButton).toBeDisabled();
  await expect(copyButton).toHaveText("Generating pairing code...");
  const loadingButton = await copyButton.elementHandle();

  await expect(copyButton).toBeEnabled();
  await expect(copyButton).toHaveText("Copy pairing code");
  expect(
    await copyButton.evaluate(
      (button, loading) => button === loading,
      loadingButton,
    ),
  ).toBe(true);
});

test("lost boot offers phone recovery with a single-use QR", async ({
  page,
}, testInfo) => {
  await installMockBridge(
    page,
    { identityLost: true },
    { skipOnboardingSeed: true },
  );
  await page.goto("/");

  await page.getByTestId("nostr-import-phone-link").click();
  await expect(page.getByTestId("identity-recovery-pairing")).toBeVisible();
  await expect(page.getByTestId("identity-recovery-qr")).toBeVisible();
  await expect(
    page.getByText("Scan this code with a signed-in Buzz phone."),
  ).toBeVisible();
  await expect(
    page.getByText("On your phone, open Settings → Send identity to desktop."),
  ).toBeVisible();
  await page.waitForTimeout(1_000); // Let the onboarding entrance motion settle.
  await page.screenshot({
    path: testInfo.outputPath("desktop-phone-recovery-qr.png"),
    fullPage: true,
  });

  const copyButton = page.getByTestId("copy-identity-recovery-code");
  await expect(copyButton).toHaveText("Copy pairing code");
  await page.context().grantPermissions(["clipboard-read", "clipboard-write"]);
  await copyButton.click();
  await expect(copyButton).toHaveText("Copied");

  const copiedPayload = await page.evaluate(() => {
    const log = (
      window as Window & {
        __BUZZ_E2E_COMMAND_LOG__?: Array<{
          command: string;
          payload: Record<string, unknown> | null;
        }>;
      }
    ).__BUZZ_E2E_COMMAND_LOG__;
    return log?.findLast(({ command }) => command === "copy_text_to_clipboard")
      ?.payload;
  });
  expect(copiedPayload?.text).toMatch(/^nostrpair:\/\/.+&mode=recover$/);

  const commands = await page.evaluate(
    () =>
      (
        window as Window & {
          __BUZZ_E2E_COMMAND_PAYLOADS__?: Array<{ command: string }>;
        }
      ).__BUZZ_E2E_COMMAND_PAYLOADS__ ?? [],
  );
  expect(
    commands.some(
      (entry) => entry.command === "start_identity_recovery_pairing",
    ),
  ).toBe(true);
});

test("phone recovery uses the desktop pairing card semantics", async ({
  page,
}) => {
  await installMockBridge(
    page,
    { identityLost: true },
    { skipOnboardingSeed: true },
  );
  await page.goto("/");

  await page.getByTestId("nostr-import-phone-link").click();
  const card = page.getByTestId("identity-recovery-pairing");
  const qrContainer = card.getByTestId("identity-recovery-qr-container");
  const qrCode = card.getByTestId("identity-recovery-qr");
  const copyButton = card.getByTestId("copy-identity-recovery-code");
  await expect(qrCode).toBeVisible();
  await expect(qrCode).toHaveAttribute("data-qr-matrix-size", "57");
  await expect(qrCode.locator("[data-qr-finder-pattern]")).toHaveCount(3);
  await expect(qrCode.locator(".buzz-qr-cell-reveal").first()).toHaveCSS(
    "animation-name",
    "buzz-qr-cell-reveal",
  );
  const qrBox = await qrContainer.boundingBox();
  const copyBox = await copyButton.boundingBox();
  expect(qrBox).not.toBeNull();
  expect(copyBox).not.toBeNull();
  expect(Math.abs((copyBox?.x ?? 0) - (qrBox?.x ?? 0))).toBeLessThan(1);
  expect(Math.abs((copyBox?.width ?? 0) - (qrBox?.width ?? 0))).toBeLessThan(1);

  await page.evaluate(async () => {
    await window.__TAURI_INTERNALS__?.invoke?.("plugin:event|emit", {
      event: "pairing-sas-received",
      payload: { sas: "123456" },
    });
  });

  await expect(
    card.getByText("Does this code match your phone?"),
  ).toBeVisible();
  await expect(
    page.getByText("Confirm the code before sharing your identity."),
  ).toBeVisible();
  await expect(
    card.getByText(
      "This gives this desktop permanent access to your Buzz identity. Only continue if you trust it.",
    ),
  ).toBeVisible();
  await expect(
    card.getByText(/On your phone, open Settings/),
  ).not.toBeVisible();
  await expect(card.getByTestId("identity-recovery-sas")).toHaveText("123 456");
  await expect(card.getByTestId("confirm-identity-recovery-sas")).toHaveText(
    "Codes match",
  );
  await expect(card.getByTestId("deny-identity-recovery-sas")).toHaveText(
    "Cancel",
  );
  const cancelBox = await card
    .getByTestId("deny-identity-recovery-sas")
    .boundingBox();
  const confirmBox = await card
    .getByTestId("confirm-identity-recovery-sas")
    .boundingBox();
  expect(cancelBox).not.toBeNull();
  expect(confirmBox).not.toBeNull();
  expect((cancelBox?.y ?? 0) - (confirmBox?.y ?? 0)).toBeGreaterThan(
    confirmBox?.height ?? 0,
  );
});

test("canceling recovery uses the standard pairing cancellation state", async ({
  page,
}) => {
  await installMockBridge(
    page,
    { identityLost: true },
    { skipOnboardingSeed: true },
  );
  await page.goto("/");
  await page.getByTestId("nostr-import-phone-link").click();
  await expect(page.getByTestId("identity-recovery-qr")).toBeVisible();

  await page.evaluate(async () => {
    await window.__TAURI_INTERNALS__?.invoke?.("plugin:event|emit", {
      event: "pairing-sas-received",
      payload: { sas: "123456" },
    });
  });
  await page.getByTestId("deny-identity-recovery-sas").click();

  await expect(
    page.getByText("The codes didn't match. Pairing was canceled."),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: "Try again" })).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).filter(
            ({ command }) => command === "cancel_pairing",
          ).length,
      ),
    )
    .toBeGreaterThan(0);
});

test("phone recovery continues to harness setup without creating or restarting", async ({
  page,
}) => {
  await installMockBridge(
    page,
    { identityLost: true },
    { skipOnboardingSeed: true },
  );
  await page.goto("/");
  await page.getByTestId("nostr-import-phone-link").click();
  await expect(page.getByTestId("identity-recovery-qr")).toBeVisible();

  await page.evaluate(async () => {
    await window.__TAURI_INTERNALS__?.invoke?.(
      "complete_identity_recovery_pairing",
    );
  });

  await expect(
    page.getByRole("heading", { name: "Set up your agent harnesses" }),
  ).toBeVisible();
  await expect(page.getByTestId("relaunch-required")).toHaveCount(0);
  await expect(
    page.getByRole("heading", {
      name: "Your unique identity key has been created",
    }),
  ).toHaveCount(0);
});

test("recovery turns relay failures into actionable copy", async ({ page }) => {
  await installMockBridge(
    page,
    { identityLost: true },
    { skipOnboardingSeed: true },
  );
  await page.goto("/");
  await page.getByTestId("nostr-import-phone-link").click();
  await expect(page.getByTestId("identity-recovery-qr")).toBeVisible();

  await page.evaluate(async () => {
    await window.__TAURI_INTERNALS__?.invoke?.("plugin:event|emit", {
      event: "pairing-error",
      payload: { message: "failed to send sas-confirm" },
    });
  });

  await expect(
    page.getByText(
      "This pairing code expired or lost its connection. Create a new code and try again.",
    ),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: "Try again" })).toBeVisible();
});

test("desktop refreshes recovery codes before the relay expires them", async ({
  page,
}) => {
  await page.clock.install();
  await installMockBridge(
    page,
    { identityLost: true },
    { skipOnboardingSeed: true },
  );
  await page.goto("/");
  await page.getByTestId("nostr-import-phone-link").click();
  await expect(page.getByTestId("identity-recovery-qr")).toBeVisible();

  const recoveryStarts = () =>
    page.evaluate(
      () =>
        (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).filter(
          ({ command }) => command === "start_identity_recovery_pairing",
        ).length,
    );
  await expect.poll(recoveryStarts).toBe(1);

  await page.clock.fastForward(90_000);
  await expect.poll(recoveryStarts).toBe(2);
  await expect(page.getByTestId("identity-recovery-qr")).toBeVisible();
});

test("importing a key from lost mode shows the relaunch-required screen", async ({
  page,
}) => {
  await installMockBridge(
    page,
    { identityLost: true },
    { skipOnboardingSeed: true },
  );
  await page.goto("/");
  await expect(
    page.getByRole("heading", { name: "Enter your private key" }),
  ).toBeVisible();

  const importedNsec = nsecEncode(hexToBytes(TEST_IDENTITIES.alice.privateKey));
  await page.getByTestId("nostr-import-nsec-input").fill(importedNsec);
  await expect(page.getByTestId("nostr-import-npub-preview")).toBeVisible();
  await page.getByTestId("nostr-import-submit").click();

  await expect(page.getByTestId("relaunch-required")).toBeVisible();
});

test("start-new-identity from lost mode persists the ephemeral key after confirmation", async ({
  page,
}) => {
  await installMockBridge(
    page,
    { identityLost: true },
    { skipOnboardingSeed: true },
  );
  await page.goto("/");
  await expect(
    page.getByRole("heading", { name: "Enter your private key" }),
  ).toBeVisible();

  page.on("dialog", (dialog) => dialog.accept());
  await page.getByRole("button", { name: "Start new identity" }).click();

  await expect(page.getByTestId("relaunch-required")).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (
            window as Window & {
              __BUZZ_E2E_COMMAND_PAYLOADS__?: Array<{ command: string }>;
            }
          ).__BUZZ_E2E_COMMAND_PAYLOADS__?.some(
            (e) => e.command === "persist_current_identity",
          ) ?? false,
      ),
    )
    .toBe(true);
});

test("cancelling start-new-identity in lost mode stays on the import screen", async ({
  page,
}) => {
  await installMockBridge(
    page,
    { identityLost: true },
    { skipOnboardingSeed: true },
  );
  await page.goto("/");
  await expect(
    page.getByRole("heading", { name: "Enter your private key" }),
  ).toBeVisible();

  page.on("dialog", (dialog) => dialog.dismiss());
  await page.getByRole("button", { name: "Start new identity" }).click();

  // Still on the import screen — no navigation, no persist
  await expect(
    page.getByRole("heading", { name: "Enter your private key" }),
  ).toBeVisible();
  await expect(page.getByTestId("relaunch-required")).toHaveCount(0);
});

test("locked boot shows the keyring-locked screen without the onboarding gate or key-import UI", async ({
  page,
}) => {
  await installMockBridge(
    page,
    { identityLocked: true },
    { skipOnboardingSeed: true },
  );
  await page.goto("/");

  await expect(page.getByTestId("keyring-locked")).toBeVisible();
  await expect(page.getByTestId("onboarding-gate")).toHaveCount(0);
  await expect(
    page.getByRole("heading", { name: "Enter your private key" }),
  ).toHaveCount(0);
});

test("locked boot can re-import a key and requires relaunch", async ({
  page,
}) => {
  await installMockBridge(
    page,
    { identityLocked: true },
    { skipOnboardingSeed: true },
  );
  await page.goto("/");

  await expect(page.getByTestId("keyring-locked")).toBeVisible();
  page.on("dialog", (dialog) => dialog.accept());
  await page
    .getByRole("button", { name: "Re-import your key instead" })
    .click();

  const importedNsec = nsecEncode(hexToBytes(TEST_IDENTITIES.alice.privateKey));
  await page.getByTestId("nostr-import-nsec-input").fill(importedNsec);
  await expect(page.getByTestId("nostr-import-npub-preview")).toBeVisible();
  await page.getByTestId("nostr-import-submit").click();

  await expect(page.getByTestId("relaunch-required")).toBeVisible();
  await expect(page.getByTestId("keyring-locked")).toHaveCount(0);
});

test("locked screen relaunch button records the process-restart invoke", async ({
  page,
}) => {
  await installMockBridge(
    page,
    { identityLocked: true },
    { skipOnboardingSeed: true },
  );
  await page.goto("/");

  await expect(page.getByTestId("keyring-locked")).toBeVisible();
  await page.getByTestId("relaunch-app").click();

  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (
            window as Window & {
              __BUZZ_E2E_COMMAND_PAYLOADS__?: Array<{ command: string }>;
            }
          ).__BUZZ_E2E_COMMAND_PAYLOADS__?.some(
            (e) => e.command === "plugin:process|restart",
          ) ?? false,
      ),
    )
    .toBe(true);
});
