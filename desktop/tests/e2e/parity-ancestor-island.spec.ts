import { expect, test } from "@playwright/test";

import { installRelayBridge } from "../helpers/bridge";
import { assertRelaySeeded } from "../helpers/seed";
import { ancestorIsland, seedScenario } from "../helpers/seedRelay";

// =============================================================================
// LIVE-RELAY parity — ancestor-island cursor poisoning (GUI read-model overhaul)
// =============================================================================
//
// The deterministic current-main RED (Dawn's Jul-2 root cause, Wren's Jul-3
// proven repro `239cc161`). The dense-second wall is NOT the disease — main
// already drains a tied second via the PR #1418 keyset fallback. THIS is the
// flaw the server-assembled windowed read model deletes:
//
//   1. Cold channel load paints the newest CHANNEL_HISTORY_LIMIT (60) rows.
//   2. One of those rows is a reply whose root/parent is OUTSIDE that window.
//   3. useLoadMissingAncestors fetches that old root by id and merges it into
//      the SAME channel cache — a non-contiguous "island" days older than the
//      contiguous window.
//   4. pageOlderMessages anchors `oldestTimestamp = baseline[0].created_at` on
//      the island, so every scroll-up pages backward FROM the island and
//      permanently skips the real history between it and the true frontier.
//   5. CLI `/query` returns those `gap-*` rows; the GUI never requests them.
//
// Contract: every top-level `gap-*` row the relay holds must be reachable in the
// timeline. RED on main (the pager anchors on the island and skips the gap
// entirely — `gap` rows reached === 0); GREEN on the windowed read model, whose
// relay-owned cursor cannot be moved by an out-of-band ancestor merge.

const RELAY_HTTP = process.env.BUZZ_E2E_RELAY_URL ?? "http://localhost:3000";

// uuid5(NAMESPACE_DNS, "buzz.channel.general") — the seeded `general` channel.
const GENERAL_CHANNEL_ID = "9f28288a-d724-587a-9709-92dc7f967110";

// >60 newest rows so the cold window (CHANNEL_HISTORY_LIMIT=60) does NOT reach
// the gap; the gap sits below the frontier, the old root below the gap.
const GAP_COUNT = 100;
const NEWEST_COUNT = 70;

test.beforeAll(async () => {
  test.setTimeout(90_000);
  await assertRelaySeeded();
});

test("live relay: an ancestor island does not strand the history frontier", async ({
  page,
}, testInfo) => {
  testInfo.setTimeout(180_000);

  // Per-run isolation: the suite seeds into shared `general`, which accumulates
  // rows across runs. A generic `gap \d+` match lets a prior run's rows inflate
  // the reachable set and false-green the parity assertion (observed: a
  // contaminated relay "passed" in 3.3s while a clean channel is RED with
  // seen.size === 0). Tag this run's rows and assert only against them.
  const nonce = `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;

  const scenario = ancestorIsland({
    channelId: GENERAL_CHANNEL_ID,
    gapCount: GAP_COUNT,
    newestCount: NEWEST_COUNT,
    nonce,
  });
  const expected = await seedScenario(scenario, { relayHttpUrl: RELAY_HTTP });
  const expectedGap = new Set(expected.map((e) => e.content));
  expect(expectedGap.size).toBe(GAP_COUNT);

  await installRelayBridge(page, "tyler");
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const timeline = page.getByTestId("message-timeline");
  await expect(timeline.locator("[data-message-id]").first()).toBeVisible();

  // Establish the poison precondition on the diseased client: on main,
  // useLoadMissingAncestors fetches the old root by id and merges it into the
  // channel cache, and "island root" appears in the timeline — that injected
  // island is what strands the pager. The overhaul DELETES useLoadMissingAncestors
  // (the relay-owned window cursor can't be moved by an out-of-band merge), so
  // the island is never injected and this row never appears. Best-effort, not a
  // hard gate: on main it settles the poison before we scroll; on the overhaul
  // it just times out harmlessly and we proceed to the reachability invariant —
  // which is the assertion that actually flips 0/100 (stranded) → 100/100.
  await timeline
    .getByText(`island root ${nonce}`, { exact: false })
    .first()
    .waitFor({ state: "visible", timeout: 15_000 })
    .catch(() => {
      /* cured client: no ancestor fetch, no island — expected on the overhaul */
    });

  // Union of THIS run's `gap` contents ever rendered. Virtualization only
  // mounts a window, so accumulate across scroll passes rather than snapshot
  // once. Match on the run nonce so a prior run's rows can't inflate the set.
  const gapPattern = new RegExp(`gap ${nonce} \\d+`);
  const renderedGapContents = async () =>
    timeline.evaluate((element, pattern: string) => {
      const re = new RegExp(pattern);
      const found: string[] = [];
      for (const row of (
        element as HTMLDivElement
      ).querySelectorAll<HTMLElement>("[data-message-id]")) {
        const match = row.textContent?.match(re);
        if (match) found.push(match[0]);
      }
      return found;
    }, gapPattern.source);

  // A real wheel-up gesture per pass: the older-history sentinel arms on a
  // genuine leave→enter transition (IntersectionObserver), so a raw scrollTop=0
  // write can fail to re-fire. A wheel event is what a real user issues.
  const wheelToTop = async () => {
    for (let step = 0; step < 12; step += 1) {
      const atTop = await timeline.evaluate(
        (element) => (element as HTMLDivElement).scrollTop <= 1,
      );
      if (atTop) break;
      await page.mouse.wheel(0, -6000);
      await page.waitForTimeout(40);
    }
  };

  const seen = new Set<string>();
  const collect = async () => {
    for (const content of await renderedGapContents()) seen.add(content);
  };

  await timeline.hover();
  let stallStreak = 0;
  for (let attempt = 0; attempt < 200 && seen.size < GAP_COUNT; attempt += 1) {
    const before = seen.size;
    await wheelToTop();
    try {
      await expect
        .poll(
          async () => {
            await collect();
            return seen.size;
          },
          { timeout: 4_000 },
        )
        .toBeGreaterThan(before);
    } catch {
      // No growth this pass — count toward a genuine stall.
    }
    await collect();
    if (seen.size > before) {
      stallStreak = 0;
    } else {
      stallStreak += 1;
      if (stallStreak > 8) break;
    }
  }

  // Parity: every gap row the relay holds must be reachable. RED on main — the
  // pager anchors on the injected island (old root) and pages backward from it,
  // skipping the entire gap span (reaches ~0 of GAP_COUNT). GREEN on the
  // windowed read model, whose relay-owned cursor an ancestor merge can't move.
  expect(seen.size).toBeGreaterThan(GAP_COUNT * 0.9);
});
