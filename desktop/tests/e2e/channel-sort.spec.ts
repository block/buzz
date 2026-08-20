import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { TEST_IDENTITIES, installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/channel-sort";

// Mock-mode current-user pubkey and relay (see e2eBridge DEFAULT_MOCK_PUBKEY /
// DEFAULT_RELAY_WS_URL). Sort preferences persist under the relay-scoped key
// buzz-channel-sort.v1:<pubkey>:<encoded-relay>.
const MOCK_PUBKEY = "deadbeef".repeat(8);
const MOCK_RELAY_ENCODED = encodeURIComponent("ws://localhost:3000");
const SORT_STORAGE_KEY = `buzz-channel-sort.v1:${MOCK_PUBKEY}:${MOCK_RELAY_ENCODED}`;

function seedSortState(page: Page, groups: Record<string, string>) {
  return page.addInitScript(
    ({ key, groups }) => {
      window.localStorage.setItem(key, JSON.stringify({ version: 1, groups }));
    },
    { key: SORT_STORAGE_KEY, groups },
  );
}

async function openApp(page: Page) {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
}

async function waitForMockLiveSubscription(page: Page, channelName: string) {
  await expect
    .poll(() =>
      page.evaluate(
        (name) =>
          window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
            channelName: name,
          }) ?? false,
        channelName,
      ),
    )
    .toBe(true);
}

async function emitMockMessage(
  page: Page,
  channelName: string,
  createdAt: number,
) {
  await page.evaluate(
    ({ channelName, createdAt, pubkey }) => {
      const emit = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
      if (!emit) {
        throw new Error("Mock message emitter is not installed");
      }
      emit({
        channelName,
        content: "Fresh activity for Recent ordering",
        createdAt,
        pubkey,
      });
    },
    { channelName, createdAt, pubkey: TEST_IDENTITIES.alice.pubkey },
  );
}

function forumNames(page: Page) {
  return page
    .getByTestId("forum-list")
    .locator("[data-testid^='channel-']")
    .evaluateAll((nodes) =>
      nodes
        .map((n) => n.getAttribute("data-testid") ?? "")
        .filter(
          (id) =>
            !id.startsWith("channel-unread") &&
            !id.startsWith("channel-working"),
        )
        .map((id) => id.replace(/^channel-/, "")),
    );
}

function streamNames(page: Page) {
  return page
    .getByTestId("stream-list")
    .locator("[data-testid^='channel-']")
    .evaluateAll((nodes) =>
      nodes
        .map((n) => n.getAttribute("data-testid") ?? "")
        .filter(
          (id) =>
            !id.startsWith("channel-unread") &&
            !id.startsWith("channel-working") &&
            !id.startsWith("channel-dm-count"),
        )
        .map((id) => id.replace(/^channel-/, "")),
    );
}

test.describe("per-group channel sort", () => {
  test("01 — Channels group defaults to A–Z", async ({ page }) => {
    await installMockBridge(page);
    await openApp(page);

    const names = await streamNames(page);
    const sorted = [...names].sort((a, b) => a.localeCompare(b));
    expect(names).toEqual(sorted);
  });

  test("02 — sort trigger switches Channels to Recent and persists", async ({
    page,
  }) => {
    await installMockBridge(page);
    await openApp(page);

    // Prime a live subscription while A–Z keeps engineering rendered, then move
    // away so the later message exercises sidebar recency rather than routing.
    await page.getByTestId("channel-engineering").click();
    await expect(page.getByTestId("chat-title")).toHaveText("engineering");
    await waitForMockLiveSubscription(page, "engineering");
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");

    // Hover the Channels header to reveal the action cluster, then open the
    // sort dropdown and choose Recent.
    const streamList = page.getByTestId("stream-list");
    await expect(streamList).toBeVisible();
    await page.getByText("Channels", { exact: true }).hover();
    const trigger = page.getByTestId("section-actions-channels");
    await expect(trigger).toBeVisible();
    await waitForAnimations(page);
    await page.screenshot({ path: `${SHOTS}/01-channels-sort-ingress.png` });
    await trigger.click();
    // Sort is now a submenu flyout — open it before the radio items render.
    await page.getByRole("menuitem", { name: "Sort" }).click();
    await expect(
      page.getByRole("menuitemradio", { name: "Recent" }),
    ).toBeVisible();
    await waitForAnimations(page);
    await page.screenshot({ path: `${SHOTS}/02-channels-sort-open.png` });
    await page.getByRole("menuitemradio", { name: "Recent" }).click();

    // Mock recency: all-replies (far future) > deep-history (1m) > general
    // (5m) > agents (15m) > sales (30m) > engineering (42m) > design (120m),
    // then no-activity channels alphabetically. The list is virtualized, so
    // only assert on the rendered prefix.
    await expect
      .poll(async () => (await streamNames(page)).slice(0, 3))
      .toEqual(["all-replies", "deep-history", "general"]);
    const names = await streamNames(page);
    const recencyOrder = [
      "all-replies",
      "deep-history",
      "general",
      "agents",
      "sales",
      "engineering",
      "design",
    ];
    const rendered = recencyOrder.filter((n) => names.includes(n));
    expect(names.slice(0, rendered.length)).toEqual(rendered);
    await waitForAnimations(page);
    await page.screenshot({ path: `${SHOTS}/03-channels-recent.png` });

    // A fresh message must advance the matching channel immediately. The mock
    // fixture intentionally pins all-replies in 2999, so use 3000 to prove the
    // live cache update outranks even that deterministic sentinel.
    await emitMockMessage(
      page,
      "engineering",
      Math.floor(Date.parse("3000-01-01T00:00:00.000Z") / 1_000),
    );
    await expect
      .poll(async () => (await streamNames(page)).slice(0, 3))
      .toEqual(["engineering", "all-replies", "deep-history"]);

    // Persisted for this identity.
    const stored = await page.evaluate((key) => {
      return JSON.parse(window.localStorage.getItem(key) ?? "null");
    }, SORT_STORAGE_KEY);
    expect(stored).toEqual({ version: 1, groups: { channels: "recent" } });

    // Survives reload.
    await page.reload();
    await expect(page.getByTestId("stream-list")).toBeVisible();
    await expect
      .poll(async () => (await streamNames(page)).slice(0, 2))
      .toEqual(["all-replies", "deep-history"]);
  });

  test("03 — group preferences are independent (seeded Channels=recent leaves Forums A–Z)", async ({
    page,
  }) => {
    await seedSortState(page, { channels: "recent" });
    await installMockBridge(page);
    await openApp(page);

    // Channels reflects the seeded Recent order…
    await expect
      .poll(async () => (await streamNames(page)).slice(0, 2))
      .toEqual(["all-replies", "deep-history"]);

    // …while Forums (unset) stays alphabetical.
    const forumOrder = await forumNames(page);
    const sortedForums = [...forumOrder].sort((a, b) => a.localeCompare(b));
    expect(forumOrder).toEqual(sortedForums);
    await waitForAnimations(page);
    await page.screenshot({ path: `${SHOTS}/04-independent-groups.png` });
  });

  test("04 — Forum Recent survives authoritative reload", async ({ page }) => {
    await seedSortState(page, { forums: "recent" });
    await installMockBridge(page);
    await openApp(page);

    await expect
      .poll(async () => (await forumNames(page)).slice(0, 2))
      .toEqual(["watercooler", "announcements"]);

    await page.reload();
    await expect(page.getByTestId("forum-list")).toBeVisible();
    await expect
      .poll(async () => (await forumNames(page)).slice(0, 2))
      .toEqual(["watercooler", "announcements"]);
  });

  test("05 — DM group has its own sort trigger", async ({ page }) => {
    await installMockBridge(page);
    await openApp(page);

    const dmList = page.getByTestId("dm-list");
    await expect(dmList).toBeVisible();
    await page.getByText("Direct messages", { exact: true }).hover();
    const trigger = page.getByTestId("section-actions-dms");
    await expect(trigger).toBeVisible();
    await trigger.click();
    // Sort is now a submenu flyout — open it before the radio items render.
    await page.getByRole("menuitem", { name: "Sort" }).click();
    await expect(
      page.getByRole("menuitemradio", { name: "A–Z" }),
    ).toBeVisible();
    await waitForAnimations(page);
    await page.screenshot({ path: `${SHOTS}/05-dm-sort-open.png` });
    await page.getByRole("menuitemradio", { name: "Recent" }).click();

    const stored = await page.evaluate((key) => {
      return JSON.parse(window.localStorage.getItem(key) ?? "null");
    }, SORT_STORAGE_KEY);
    expect(stored).toEqual({ version: 1, groups: { dms: "recent" } });
  });
});
