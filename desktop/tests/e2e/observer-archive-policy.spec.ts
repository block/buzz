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
    // Simulate a user who previously clicked OFF: the identity-scoped opt-out
    // is recorded in localStorage ("0") and the owner_p/24200 subscription row
    // is absent.  Reconciliation must honour the stored choice and leave the
    // toggle unchecked (user can re-enable via the toggle).
    const MOCK_PUBKEY = "deadbeef".repeat(8);
    await page.addInitScript(
      ({ storageKey }) => {
        window.localStorage.setItem(storageKey, "0");
      },
      {
        storageKey: `buzz:observer-archive-default-seeded:${MOCK_PUBKEY}`,
      },
    );

    await installMockBridge(page, {
      saveSubscriptions: [],
    });

    const card = await openLocalArchiveSettings(page);
    const toggle = card.getByTestId("local-archive-observer-toggle");
    await expect(toggle).toBeVisible({ timeout: 5_000 });
    await expect(toggle).toBeEnabled();
    await expect(toggle).not.toBeChecked();
  });

  test("no subscriptions, no stored choice: defaults ON then OFF removes, ON re-creates", async ({
    page,
  }) => {
    // A fresh identity with no stored choice and an empty subscription table
    // must be seeded to ON by reconciliation.  Thereafter the toggle must
    // function: OFF removes kind 24200, ON re-creates it.
    await installMockBridge(page, {
      saveSubscriptions: [],
    });

    const card = await openLocalArchiveSettings(page);
    const toggle = card.getByTestId("local-archive-observer-toggle");
    await expect(toggle).toBeVisible({ timeout: 5_000 });

    // Default-on: reconciliation seeds the row, toggle must be checked.
    await expect(toggle).toBeChecked();

    // OFF: removes kind 24200.
    await toggle.click();
    await expect(toggle).not.toBeChecked();

    // ON again: re-creates the row from empty.
    await toggle.click();
    await expect(toggle).toBeChecked();
  });
});

test.describe("observer archive policy — reconciliation gate", () => {
  test("archive sync starts only after reconciliation seeds the subscription", async ({
    page,
  }) => {
    // The gate invariant: no archive listener may open before observer
    // reconciliation has seeded kind 24200. Kind 24200 is relay-ephemeral, so
    // a listener opened early misses frames permanently.
    //
    // The archive subscription and its `#p` + kind-24200 REQ now live entirely
    // in Rust, so this spec asserts the gate at the seam that is still in JS:
    // the `start_archive_sync` start signal, which the renderer emits only
    // after reconciliation resolves. Ownership of the invariants this test
    // used to cover:
    //
    //   REQ shape (`#p` tag key, live tail) →
    //     native_relay_client::relay_backed_tests::
    //     archive_sync_session_receives_live_events_from_a_real_relay
    //     (relay-backed; `#p`→`#e` negative control confirms it is bound to
    //     the tag key)
    //   gate closed on pending/failed reconciliation →
    //     src/features/local-archive/useArchiveSync.test.mjs
    //     (mutation-verified: deleting the gate fails 3/3 there)
    //
    // Deliberately NOT re-asserted through the mock bridge: the bridge has no
    // Rust task to drive, so making it synthesize the REQ would assert the
    // mock's own re-implementation rather than the shipped behavior.
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

    const readCommands = () =>
      page.evaluate(
        () =>
          (window as Window & { __BUZZ_E2E_COMMANDS__?: string[] })
            .__BUZZ_E2E_COMMANDS__ ?? [],
      );

    await expect
      .poll(async () => (await readCommands()).includes("start_archive_sync"), {
        timeout: 10_000,
      })
      .toBe(true);

    const commands = await readCommands();

    // What this e2e uniquely owns is WIRING: that AppShell actually mounts
    // `useArchiveSync` and its start signal reaches the IPC boundary. Drop
    // the hook from AppShell and archive never starts in the shipped app,
    // and no unit test notices — they mount the hook directly.
    //
    // It deliberately does NOT assert gate ORDERING. The bridge records a
    // command when it is invoked, not when it resolves, and reconciliation's
    // `merge_save_subscription_kinds` is invoked synchronously on mount — so
    // `merge` precedes `start` in this log even with the `ready` gate deleted.
    // Verified by mutation: removing the gate leaves this spec 6/6 green while
    // `useArchiveSync.test.mjs` fails 3/3. The gate is asserted there, where
    // the assertion has teeth; asserting it here would only look like coverage.
    expect(commands).toContain("merge_save_subscription_kinds");
    expect(commands).toContain("start_archive_sync");

    // The epoch handshake, asserted on the wire rather than by construction.
    // The renderer must announce and AWAIT its realm epoch before it may issue
    // any lifecycle command, and must pass that epoch through. Nothing here
    // validates types at runtime — the typed wrapper is erased — so a bridge
    // returning null would flow null into `startArchiveSync(null, lease)` and
    // this no-op bridge would accept it. Asserting only that the command was
    // named passes either way; asserting order and payload is what makes this
    // a receipt.
    const payloads = await page.evaluate(
      () =>
        (
          window as Window & {
            __BUZZ_E2E_COMMAND_PAYLOADS__?: Array<{
              command: string;
              payload: unknown;
            }>;
          }
        ).__BUZZ_E2E_COMMAND_PAYLOADS__ ?? [],
    );
    const announcedAt = payloads.findIndex(
      (entry) => entry.command === "announce_archive_sync_epoch",
    );
    const startedAt = payloads.findIndex(
      (entry) => entry.command === "start_archive_sync",
    );
    expect(announcedAt).toBeGreaterThanOrEqual(0);
    expect(startedAt).toBeGreaterThan(announcedAt);
    expect(payloads[startedAt]?.payload).toMatchObject({
      epoch: expect.any(Number),
      lease: expect.any(Number),
    });
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

    // The seeded row itself, read back through `list_save_subscriptions` —
    // the state the repair path is supposed to produce. This replaces a poll
    // on the live `#p`+24200 REQ: that REQ is now opened by the Rust archive
    // task, which the mock bridge never runs. Seeding is unchanged by that
    // move and stays asserted here.
    await expect
      .poll(
        () =>
          page.evaluate(async (ownerPubkey) => {
            const invoke = (
              window as Window & {
                __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
                  command: string,
                  payload?: unknown,
                ) => Promise<unknown>;
              }
            ).__BUZZ_E2E_INVOKE_MOCK_COMMAND__;
            if (!invoke) return false;
            const rows = (await invoke("list_save_subscriptions")) as Array<{
              scope_type: string;
              scope_value: string;
              kinds: string;
            }>;
            return rows.some(
              (row) =>
                row.scope_type === "owner_p" &&
                row.scope_value === ownerPubkey &&
                (JSON.parse(row.kinds) as number[]).includes(24200),
            );
          }, "deadbeef".repeat(8)),
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
