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
  test("internal policy: toggle disabled with policy-locked copy", async ({
    page,
  }) => {
    await installMockBridge(page, {
      observerArchiveDefaultEnabled: true,
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
    await expect(toggle).toBeDisabled();
    await expect(
      card.getByText(/always on for internal builds/i),
    ).toBeVisible();
  });

  test("OSS policy: toggle is functional", async ({ page }) => {
    await installMockBridge(page, {
      observerArchiveDefaultEnabled: false,
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

  test("OSS policy, no subscriptions: toggle enabled and unchecked", async ({
    page,
  }) => {
    // Resolved-OSS empty-subscription state: no owner_p/24200 row exists,
    // so the toggle reads unchecked, and OSS policy (false) keeps it
    // enabled — confirming fail-closed doesn't permanently lock OSS users
    // out once the policy flag resolves.
    await installMockBridge(page, {
      observerArchiveDefaultEnabled: false,
      saveSubscriptions: [],
    });

    const card = await openLocalArchiveSettings(page);
    const toggle = card.getByTestId("local-archive-observer-toggle");
    await expect(toggle).toBeVisible({ timeout: 5_000 });
    await expect(toggle).toBeEnabled();
    await expect(toggle).not.toBeChecked();
  });

  test("policy pending: toggle disabled, then enabled once resolved", async ({
    page,
  }) => {
    await installMockBridge(page, {
      observerArchiveDefaultEnabled: false,
      observerArchiveDefaultEnabledDelayMs: 500,
      saveSubscriptions: [],
    });

    const card = await openLocalArchiveSettings(page);
    const toggle = card.getByTestId("local-archive-observer-toggle");
    await expect(toggle).toBeVisible({ timeout: 5_000 });
    // Fail-closed: disabled while the policy check is still in flight.
    await expect(toggle).toBeDisabled();
    await expect(toggle).toBeEnabled({ timeout: 5_000 });
    await expect(toggle).not.toBeChecked();
  });

  test("policy check fails: toggle stays disabled and issues no mutation", async ({
    page,
  }) => {
    await installMockBridge(page, {
      observerArchiveDefaultEnabled: false,
      observerArchiveDefaultEnabledError: "policy check failed",
      saveSubscriptions: [],
    });

    const card = await openLocalArchiveSettings(page);
    const toggle = card.getByTestId("local-archive-observer-toggle");
    await expect(toggle).toBeVisible({ timeout: 5_000 });
    // Rejection leaves `observerPolicy` at its initial `undefined` — the
    // fail-closed `.catch()` in LocalArchiveSettingsCard must not flip it
    // to a permissive state. Give the rejection time to settle, then
    // assert the disabled state holds (not just "hasn't flipped yet").
    await page.waitForTimeout(200);
    await expect(toggle).toBeDisabled();

    const commands = await page.evaluate(
      () =>
        (window as Window & { __BUZZ_E2E_COMMANDS__?: string[] })
          .__BUZZ_E2E_COMMANDS__ ?? [],
    );
    expect(
      commands.filter(
        (c) =>
          c === "merge_save_subscription_kinds" ||
          c === "remove_save_subscription_kind",
      ),
    ).toEqual([]);
  });

  test("OSS policy: toggle click ON merges kind 24200, click OFF removes the row", async ({
    page,
  }) => {
    await installMockBridge(page, {
      observerArchiveDefaultEnabled: false,
      saveSubscriptions: [],
    });

    const card = await openLocalArchiveSettings(page);
    const toggle = card.getByTestId("local-archive-observer-toggle");
    await expect(toggle).toBeVisible({ timeout: 5_000 });
    await expect(toggle).not.toBeChecked();

    // ON: merges kind 24200 into a fresh owner_p row (the row-creation edge
    // of merge_save_subscription_kinds).
    await toggle.click();
    await expect(toggle).toBeChecked();

    // OFF: removes kind 24200. Since it's the row's only kind, the row is
    // deleted entirely (remove_save_subscription_kind's row-delete-on-empty
    // edge) — re-checking observerEnabled must correctly read "no row" as
    // unchecked, not stale/checked.
    await toggle.click();
    await expect(toggle).not.toBeChecked();

    // ON again: re-creates the row from empty, proving the delete above was
    // a real row removal and not a lingering empty-kinds row.
    await toggle.click();
    await expect(toggle).toBeChecked();
  });
});

test.describe("observer archive policy — reconciliation gate", () => {
  test("internal policy: archive sync reaches subscription path after reconciliation", async ({
    page,
  }) => {
    await installMockBridge(page, {
      observerArchiveDefaultEnabled: true,
      saveSubscriptions: [
        {
          scope_type: "owner_p",
          scope_value: "deadbeef".repeat(8),
          kinds: "[24200]",
        },
      ],
    });

    await page.goto("/", { waitUntil: "domcontentloaded" });

    // Wait for the channel list to appear (proves AppShell mounted fully).
    await expect(page.getByTestId("channel-general")).toBeVisible({
      timeout: 10_000,
    });

    // The reconciliation gate (useObserverArchiveReconciliation) must have
    // resolved successfully, allowing useArchiveSync to start the
    // ArchiveSyncManager, which calls list_save_subscriptions. The IPC
    // counter proves the subscription path was reached.
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

    // Bonus (Thufir pass 2, F4): the reconciliation gate must also result
    // in a real `#p` + kind-24200 live REQ filter, not just an IPC call.
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

  test("policy pending: no subscription list call or live filter until resolved", async ({
    page,
  }) => {
    await installMockBridge(page, {
      observerArchiveDefaultEnabled: true,
      observerArchiveDefaultEnabledDelayMs: 500,
      saveSubscriptions: [
        {
          scope_type: "owner_p",
          scope_value: "deadbeef".repeat(8),
          kinds: "[24200]",
        },
      ],
    });

    await page.goto("/", { waitUntil: "domcontentloaded" });
    await expect(page.getByTestId("channel-general")).toBeVisible({
      timeout: 10_000,
    });

    // While the policy check is pending, useArchiveSync must not have
    // started — no list_save_subscriptions call, no owner/24200 live
    // filter. This is the discriminating half pass 2 found missing: the
    // prior test only proved "eventually starts", not "doesn't start
    // early."
    const countWhilePending = await page.evaluate(
      () =>
        (
          (window as Record<string, unknown>).__BUZZ_E2E_IPC_COUNTERS__ as
            | Record<string, number>
            | undefined
        )?.list_save_subscriptions ?? 0,
    );
    expect(countWhilePending).toBe(0);
    const hasSubscriptionWhilePending = await page.evaluate(
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
    expect(hasSubscriptionWhilePending).toBe(false);

    // After the policy resolves, both the IPC call and the live filter
    // appear.
    await page.waitForFunction(
      () =>
        ((
          (window as Record<string, unknown>).__BUZZ_E2E_IPC_COUNTERS__ as
            | Record<string, number>
            | undefined
        )?.list_save_subscriptions ?? 0) > 0,
      null,
      { timeout: 10_000 },
    );
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
        { timeout: 5_000 },
      )
      .toBe(true);
  });

  test("policy check fails: subscription path never opens", async ({
    page,
  }) => {
    await installMockBridge(page, {
      observerArchiveDefaultEnabled: true,
      observerArchiveDefaultEnabledError: "policy check failed",
      saveSubscriptions: [
        {
          scope_type: "owner_p",
          scope_value: "deadbeef".repeat(8),
          kinds: "[24200]",
        },
      ],
    });

    await page.goto("/", { waitUntil: "domcontentloaded" });
    await expect(page.getByTestId("channel-general")).toBeVisible({
      timeout: 10_000,
    });

    // Give the rejected reconciliation time to settle, then assert the
    // gate stayed shut: no list_save_subscriptions call, no live filter.
    await page.waitForTimeout(500);
    const count = await page.evaluate(
      () =>
        (
          (window as Record<string, unknown>).__BUZZ_E2E_IPC_COUNTERS__ as
            | Record<string, number>
            | undefined
        )?.list_save_subscriptions ?? 0,
    );
    expect(count).toBe(0);
    const hasSubscription = await page.evaluate(
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
    expect(hasSubscription).toBe(false);
  });

  test("fresh internal install: reconciliation repairs an empty subscription list", async ({
    page,
  }) => {
    // The actual production repair path Will's bug report was about: a
    // fresh internal install with no owner_p/24200 row yet must end up
    // with one after startup reconciliation runs — not just "no-op
    // because the row was already there" (the prior fixture always
    // pre-seeded the row).
    await installMockBridge(page, {
      observerArchiveDefaultEnabled: true,
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
