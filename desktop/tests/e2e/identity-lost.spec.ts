import { hexToBytes } from "@noble/hashes/utils.js";
import { expect, test } from "@playwright/test";
import { nsecEncode } from "nostr-tools/nip19";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

test("normal first launch uses the already-persisted identity", async ({
  page,
}) => {
  await installMockBridge(page, undefined, {
    skipCommunitySeed: true,
    skipOnboardingSeed: true,
  });
  await page.goto("/");

  await expect(page.getByTestId("machine-onboarding-gate")).toBeVisible();
  await page.getByRole("button", { name: "Get started" }).click();

  await expect(
    page.getByRole("heading", { name: "Save your private key" }),
  ).toBeVisible();
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
}) => {
  await installMockBridge(
    page,
    { identityLost: true },
    { skipOnboardingSeed: true },
  );
  await page.goto("/");

  await expect(page.getByTestId("machine-onboarding-gate")).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Re-import your key" }),
  ).toBeVisible();
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
    page.getByRole("heading", { name: "Re-import your key" }),
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
    page.getByRole("heading", { name: "Re-import your key" }),
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
    page.getByRole("heading", { name: "Re-import your key" }),
  ).toBeVisible();

  page.on("dialog", (dialog) => dialog.dismiss());
  await page.getByRole("button", { name: "Start new identity" }).click();

  // Still on the import screen — no navigation, no persist
  await expect(
    page.getByRole("heading", { name: "Re-import your key" }),
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
    page.getByRole("heading", { name: "Re-import your key" }),
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
