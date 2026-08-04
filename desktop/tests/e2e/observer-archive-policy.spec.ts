import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

async function openLocalArchiveSettings(page: import("@playwright/test").Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-settings").click();
  await page.getByTestId("profile-popover-settings").click();
  await expect(page.getByTestId("settings-view")).toBeVisible();
  await page.getByTestId("settings-nav-local-archive").click();
  const card = page.getByTestId("settings-local-archive");
  await expect(card).toBeVisible({ timeout: 10_000 });
  return card;
}

test.describe("observer archive policy — Settings toggle", () => {
  test("fresh identity: observer toggle is enabled and checked by default", async ({
    page,
  }) => {
    // Archive is default-on for all builds. A fresh identity (no stored opt-out)
    // should show the toggle enabled and checked after reconciliation seeds the
    // kind-24200 subscription.
    await installMockBridge(page, {
      saveSubscriptions: [
        {
          scope_type: "owner_p",
          scope_value: "deadbeef".repeat(8),
          kinds: "[24200]",
        },
      ],
    });

    const card = await openLocalArchiveSettings(page);
    const toggle = card.getByTestId("local-archive-observer-toggle");
    await expect(toggle).toBeVisible({ timeout: 5_000 });
    await expect(toggle).toBeEnabled();
    await expect(toggle).toBeChecked();
  });

  test("toggle click OFF disables, then ON re-enables", async ({ page }) => {
    await installMockBridge(page, {
      saveSubscriptions: [
        {
          scope_type: "owner_p",
          scope_value: "deadbeef".repeat(8),
          kinds: "[24200]",
        },
      ],
    });

    const card = await openLocalArchiveSettings(page);
    const toggle = card.getByTestId("local-archive-observer-toggle");
    await expect(toggle).toBeVisible({ timeout: 5_000 });
    await expect(toggle).toBeChecked();

    // OFF: removes kind 24200.
    await toggle.click();
    await expect(toggle).not.toBeChecked();

    // ON again: re-creates the row from empty.
    await toggle.click();
    await expect(toggle).toBeChecked();
  });

  test("explicit opt-out persists across reload: toggle stays OFF", async ({
    page,
  }) => {
    // Simulate a user who already clicked OFF and whose opt-out is persisted
    // (no owner_p/24200 row in the DB). The toggle must remain unchecked
    // and enabled (user can re-enable) rather than re-seeding on load.
    await installMockBridge(page, {
      saveSubscriptions: [],
    });

    const card = await openLocalArchiveSettings(page);
    const toggle = card.getByTestId("local-archive-observer-toggle");
    await expect(toggle).toBeVisible({ timeout: 5_000 });
    await expect(toggle).toBeEnabled();
    await expect(toggle).not.toBeChecked();
  });

  test("no subscriptions: toggle ON merges kind 24200, toggle OFF removes it", async ({
    page,
  }) => {
    await installMockBridge(page, {
      saveSubscriptions: [],
    });

    const card = await openLocalArchiveSettings(page);
    const toggle = card.getByTestId("local-archive-observer-toggle");
    await expect(toggle).toBeVisible({ timeout: 5_000 });
    await expect(toggle).not.toBeChecked();

    // ON: merges kind 24200 into a fresh owner_p row.
    await toggle.click();
    await expect(toggle).toBeChecked();

    // OFF: removes kind 24200 (row-delete-on-empty edge).
    await toggle.click();
    await expect(toggle).not.toBeChecked();

    // ON again: re-creates the row, proving the delete was real.
    await toggle.click();
    await expect(toggle).toBeChecked();
  });
});

test.describe("observer archive policy — reconciliation gate", () => {
  test("archive sync reaches subscription path after reconciliation", async ({
    page,
  }) => {
    // The reconciliation gate (useObserverArchiveReconciliation) must resolve
    // successfully for a fresh identity, allowing useArchiveSync to start the
    // ArchiveSyncManager, which calls list_save_subscriptions.
    await installMockBridge(page, {
      saveSubscriptions: [
        {
          scope_type: "owner_p",
          scope_value: "deadbeef".repeat(8),
          kinds: "[24200]",
        },
      ],
    });

    await page.goto("/", { waitUntil: "domcontentloaded" });

    // Wait for the channel list (proves AppShell mounted fully).
    await expect(page.getByTestId("channel-general")).toBeVisible({
      timeout: 10_000,
    });

    // The IPC counter proves the subscription path was reached.
    await page.waitForFunction(
      () => {
        const counters = (window as Record<string, unknown>)
          .__BUZZ_E2E_IPC_COUNTERS__ as Record<string, number> | undefined;
        return (counters?.list_save_subscriptions ?? 0) > 0;
      },
      null,
      { timeout: 10_000 },
    );

    const count = await page.evaluate(() => {
      const counters = (window as Record<string, unknown>)
        .__BUZZ_E2E_IPC_COUNTERS__ as Record<string, number> | undefined;
      return counters?.list_save_subscriptions ?? 0;
    });
    expect(count).toBeGreaterThan(0);

    // The reconciliation gate must also result in a real `#p` + kind-24200
    // live REQ filter.
    const hasOwnerKindSubscription = await page.evaluate(
      (ownerPubkey) =>
        (
          window as Window & {
            __BUZZ_E2E_HAS_MOCK_OWNER_KIND_SUBSCRIPTION__?: (input: {
              ownerPubkey: string;
              kind: number;
            }) => boolean;
          }
        ).__BUZZ_E2E_HAS_MOCK_OWNER_KIND_SUBSCRIPTION__?.({
          ownerPubkey,
          kind: 24200,
        }) ?? false,
      "deadbeef".repeat(8),
    );
    expect(hasOwnerKindSubscription).toBe(true);
  });

  test("fresh install with empty subscriptions: reconciliation seeds kind 24200", async ({
    page,
  }) => {
    // A fresh install with no owner_p/24200 row must end up with one after
    // startup reconciliation runs — the actual production repair path.
    await installMockBridge(page, {
      saveSubscriptions: [],
    });

    await page.goto("/", { waitUntil: "domcontentloaded" });
    await expect(page.getByTestId("channel-general")).toBeVisible({
      timeout: 10_000,
    });

    await expect
      .poll(
        () =>
          page.evaluate(
            (ownerPubkey) =>
              (
                window as Window & {
                  __BUZZ_E2E_HAS_MOCK_OWNER_KIND_SUBSCRIPTION__?: (input: {
                    ownerPubkey: string;
                    kind: number;
                  }) => boolean;
                }
              ).__BUZZ_E2E_HAS_MOCK_OWNER_KIND_SUBSCRIPTION__?.({
                ownerPubkey,
                kind: 24200,
              }) ?? false,
            "deadbeef".repeat(8),
          ),
        { timeout: 10_000 },
      )
      .toBe(true);

    const commands = await page.evaluate(
      () =>
        (window as Window & { __BUZZ_E2E_COMMANDS__?: string[] })
          .__BUZZ_E2E_COMMANDS__ ?? [],
    );
    expect(commands).toContain("merge_save_subscription_kinds");
  });
});
