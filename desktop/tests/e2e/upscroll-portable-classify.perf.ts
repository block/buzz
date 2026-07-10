import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

/**
 * PORTABLE reversal classifier — implementation-agnostic scroll-anchoring judge.
 *
 * WHY THIS EXISTS (thread root 0a496379 / c62888de, Eva's kickoff). The W4a
 * classifier (`upscroll-fast-classify.perf.ts`, on the W-branches) types every
 * upscroll reversal by reading `window.__ANCHOR_PROBE__` — the per-rAF probe
 * that `useAnchoredScroll` emits (`wouldFire`, `signedShift`, `renderedScroll`,
 * `source`). Those are OUR corrector's internal quantities. Point that fixture
 * at a virtua / react-virtuoso prototype and it scores nothing: a virtualizer
 * that owns anchoring internally emits no such probe. It cannot even run on our
 * OWN unmodified `main` — the probe is a W-branch instrument, absent here.
 *
 * For the SOTA comparison Tyler asked for (ours today vs a packaged virtualizer
 * tomorrow) the instrument must read only what ANY timeline exposes to the DOM:
 * a scroll container, rows with a stable id, and layout. This fixture rekeys the
 * whole methodology onto pure DOM observables so the SAME corpus + SAME reversal
 * detector + SAME cause axis judge our impl and any candidate apples-to-apples.
 *
 * WHAT PORTS UNCHANGED (already pure DOM in the W4a fixture):
 *   • reversal DETECTION — rowMove = rect.top(b) − rect.top(a), dScroll =
 *     scrollTop(b) − scrollTop(a), dev = rowMove − dScroll. All getBoundingClientRect
 *     + scrollTop. A reversal is rowMove <= −REVERSAL_PX (row moved against the
 *     upscroll by more than staircase noise).
 *
 * WHAT IS RE-DERIVED FROM LAYOUT (replaces the hook probe's grow/shrink/skip):
 *   The W4a classes answered "why did OUR corrector produce this reversal" —
 *   meaningless for an impl with no corrector. The portable cause axis asks the
 *   impl-neutral question instead: did content ABOVE the anchor reflow?
 *     • aboveΔ = anchor.offsetTop(b) − anchor.offsetTop(a). A row's offsetTop
 *       inside the scroll content is INVARIANT under scroll and changes ONLY
 *       when layout above it changes height. So aboveΔ isolates the reflow's
 *       push on the anchor from the wheel's scroll — no hook needed.
 *     • REFLOW-DRIVEN reversal (|aboveΔ| > EPS): content above grew/shrank and
 *       the impl failed to compensate the push. Our GROW/SHRINK collapse here;
 *       a virtualizer's blank-flash-on-resize lands here too. rowMove tracks
 *       aboveΔ.
 *     • TRACKING-FAILURE reversal (|aboveΔ| ≈ 0): no layout moved the anchor,
 *       yet the row reversed against scroll. This is the scroll-anchoring /
 *       compositor-desync class we characterized on WebKit (rowMove reverses
 *       while offsetTop is still) — the failure a transform-positioned
 *       virtualizer is HYPOTHESISED to sidestep. If a candidate zeroes this
 *       column on WebKit, that's the headline.
 *
 * VERDICT COLUMNS (per engine): reversal count, max reversal px, still-frame
 * count (|dScroll| < REVERSAL_PX — the felt case, eye barely moving), the
 * reflow-driven / tracking-failure split, the mid-momentum one-way regression
 * guard, and the post-momentum bite count + its restricted max-px (THE
 * discriminator, co-gated: pass = bite count AND bite max-px both down). Same
 * numbers for every impl.
 *
 * HONESTY BOUND (carried from the W4a fixture): Playwright `mouse.wheel` is a
 * synthetic discrete event; the real WKWebView coalesced-momentum still frame
 * is a device phenomenon that only fully reproduces in the shipped Tauri shell
 * (Tyler's real-trackpad pass is the acceptance gate). But this corpus DOES
 * surface the bounded WebKit survivors under Playwright WebKit, so the fixture
 * reproduces the frames a candidate must be judged on. It CHARACTERIZES for
 * comparison; it does not gate a ceiling.
 *
 * NO BUILD STAMP. The W4a fixture asserts `__ANCHOR_BUILD_STAMP__` to guard
 * against a stale bundle of the hook fix. This fixture touches no hook internal,
 * so there is nothing impl-specific to stamp — it runs against whatever timeline
 * the build ships, which is the whole point.
 */

// Heterogeneous rows so estimateRowHeight (and any candidate's height estimator)
// mis-reserves most of them — the reserve-vs-true error that makes above-anchor
// reflow happen on scroll. Self-contained: seeded via the live-emit bridge into
// `general`, so this fixture carries its own corpus and needs no mock channel.
const SEED_ROWS = 500;
const WHEEL_DELTA = 12; // px/event — matches the W4a gate's constant velocity
const WHEEL_PERIOD_MS = 32; // ~375px/s
const DURATION_MS = 12_000;
const SAFE_MARGIN = 100; // anchor must sit this far inside the viewport band
const REVERSAL_PX = 3; // against-scroll move past staircase noise
const ABOVE_EPS = 1; // px: |aboveΔ| below this = no reflow above the anchor

type Frame = {
  t: number;
  top: number | null; // anchor rect.top (viewport), or null when out of band
  offsetTop: number; // anchor offsetTop within scroll content — reflow probe
  scrollTop: number;
  rowId: string | null;
};

test("PORTABLE upscroll classify: reflow-driven vs tracking-failure reversals", async ({
  page,
  browserName,
}) => {
  await installMockBridge(page);
  test.setTimeout(120_000); // 500-row seed + 12s wheel drive + WebKit settle
  page.on("console", (m) => {
    if (m.type() === "error") console.log("PAGE ERROR:", m.text());
  });
  page.on("pageerror", (e) => console.log("PAGE EXCEPTION:", e.message));
  await page.goto("/");
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
  );

  // Open the channel FIRST, then live-emit the corpus. EMIT_MOCK_MESSAGE is a
  // LIVE relay event: the timeline only renders it once the channel is open and
  // subscribed. (Emitting before open lands the rows in the store but the
  // initial paged history fetch caps well below the corpus size — the row-mount
  // wait then never reaches threshold, which is why the pre-existing
  // scroll-smoothness fixture times out on main too.) Opening first makes every
  // emit paint into the live DOM, which is the un-virtualized list under test.
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  const timeline = page.getByTestId("message-timeline");

  await page.evaluate((rows: number) => {
    // Six structurally distinct row kinds cycled by index — the same
    // heterogeneity class the W4a jitter-corpus used, inlined here so the
    // fixture carries its own corpus (needs no mock channel on main).
    const heteroBody = (i: number): string => {
      switch (i % 6) {
        case 0:
          return `quick note ${i}`;
        case 1:
          return `# Heading ${i}\n\n- alpha ${i}\n- beta ${i}\n- gamma ${i}\n- delta ${i}`;
        case 2:
          return `This is a deliberately long paragraph number ${i} that wraps across several visual lines depending on the true column width, which a fixed characters-per-line heuristic only approximates and therefore systematically mis-estimates on wide and narrow windows alike.`;
        case 3:
          return `> quoted reply ${i}\n>\n> second quoted line ${i}\n\nfollow-up prose ${i}`;
        case 4:
          return `runtime log ${i}:\n\`\`\`\nconst x = ${i};\nconsole.log(x * 2);\nreturn x;\n\`\`\``;
        default:
          return `paragraph one for row ${i}\n\nparagraph two for row ${i} with a little more text to push it onto a second wrapped line`;
      }
    };
    for (let i = 0; i < rows; i += 1) {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: heteroBody(i),
      });
    }
  }, SEED_ROWS);

  // Impl-agnostic readiness. The OLD wait ("> 50 mounted [data-message-id]")
  // baked in an un-virtualized-DOM assumption: it only passes when the impl
  // mounts every visible-band row into the DOM at once. A real virtualizer
  // WINDOWS the mount set below 50 by design (that is the whole point), so the
  // wait times out on a correct candidate and never yields a perf number — the
  // exact impl-coupling this fixture's rekey exists to remove. Replace it with
  // the signal ANY timeline exposes once its corpus is loaded and scrollable:
  //   (1) the scroll container overflows its own client box by a wide margin —
  //       500 heterogeneous rows produce many screens of content, so a scroll
  //       container whose scrollHeight is a large multiple of its clientHeight
  //       has clearly ingested the corpus, whether it mounts 12 rows or 500. A
  //       generous multiple (not a fixed px total) keeps this height-estimator
  //       neutral: a windowed impl reports its FULL estimated scrollHeight even
  //       while mounting a fraction of the rows.
  //   (2) at least one message row is present — the sampler needs a live anchor.
  // Zero per-impl branches: both conditions read only DOM every timeline exposes.
  const MIN_OVERFLOW_RATIO = 8; // scrollHeight must exceed clientHeight by >=8x
  await page.waitForFunction((minRatio) => {
    const el = document.querySelector(
      '[data-testid="message-timeline"]',
    ) as HTMLDivElement | null;
    if (!el) return false;
    const overflows =
      el.clientHeight > 0 && el.scrollHeight >= el.clientHeight * minRatio;
    const hasRow = el.querySelectorAll("[data-message-id]").length >= 1;
    return overflows && hasRow;
  }, MIN_OVERFLOW_RATIO);
  await page.waitForTimeout(500); // let live emits settle

  await timeline.evaluate((element) => {
    const el = element as HTMLDivElement;
    el.style.overflowAnchor = "none";
    el.scrollTop = el.scrollHeight;
    el.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  await page.waitForTimeout(200);
  await timeline.hover();

  // Per-frame sampler — pure DOM. Tracks one anchor row's viewport rect.top AND
  // its offsetTop inside the scroll content. offsetTop is scroll-invariant, so
  // its per-frame delta is the above-anchor reflow, isolated from the wheel.
  await timeline.evaluate((element, margin: number) => {
    const el = element as HTMLDivElement;
    const w = window as unknown as {
      __PCLASS__: { frames: Frame[]; stop: boolean };
    };
    type Frame = {
      t: number;
      top: number | null;
      offsetTop: number;
      scrollTop: number;
      rowId: string | null;
    };
    w.__PCLASS__ = { frames: [], stop: false };
    let trackedId: string | null = null;
    const pick = (): string | null => {
      const box = el.getBoundingClientRect();
      for (const row of el.querySelectorAll<HTMLElement>("[data-message-id]")) {
        const rect = row.getBoundingClientRect();
        if (rect.top > box.top + margin && rect.bottom < box.bottom - margin) {
          return row.dataset.messageId ?? null;
        }
      }
      return null;
    };
    const tick = (t: number) => {
      if (w.__PCLASS__.stop) return;
      let top: number | null = null;
      let offsetTop = 0;
      if (trackedId) {
        const row = el.querySelector<HTMLElement>(
          `[data-message-id="${CSS.escape(trackedId)}"]`,
        );
        if (row) {
          const rect = row.getBoundingClientRect();
          const box = el.getBoundingClientRect();
          const inBand =
            rect.top > box.top + margin && rect.bottom < box.bottom - margin;
          top = inBand ? rect.top : null;
          offsetTop = row.offsetTop;
        }
      }
      if (top === null) trackedId = pick();
      w.__PCLASS__.frames.push({
        t,
        top,
        offsetTop,
        scrollTop: el.scrollTop,
        rowId: trackedId,
      });
      requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  }, SAFE_MARGIN);

  // Constant upscroll drive — surfaces the bounded WebKit survivor corpus.
  const started = Date.now();
  while (Date.now() - started < DURATION_MS) {
    await page.mouse.wheel(0, -WHEEL_DELTA);
    await page.waitForTimeout(WHEEL_PERIOD_MS);
  }

  const frames: Frame[] = await timeline.evaluate(() => {
    const w = window as unknown as {
      __PCLASS__: { frames: Frame[]; stop: boolean };
    };
    w.__PCLASS__.stop = true;
    return w.__PCLASS__.frames;
  });

  // Score same-anchor frame pairs. A reversal is rowMove <= −REVERSAL_PX. Cause
  // is read from aboveΔ (offsetTop delta): reflow-driven if content above moved,
  // tracking-failure if it did not. No hook, no probe — the same axis works on
  // any impl that renders rows into a scroll container.
  type Cause = "reflow-driven" | "tracking-failure";
  let scored = 0;
  let reanchors = 0;
  const reversals: Array<{
    i: number;
    rowMove: number;
    dScroll: number;
    dev: number;
    aboveDelta: number;
    cause: Cause;
    rowId: string | null;
  }> = [];
  for (let i = 1; i < frames.length; i += 1) {
    const a = frames[i - 1];
    const b = frames[i];
    if (
      a.top === null ||
      b.top === null ||
      a.rowId === null ||
      a.rowId !== b.rowId
    ) {
      reanchors += 1;
      continue;
    }
    scored += 1;
    const rowMove = b.top - a.top;
    if (rowMove > -REVERSAL_PX) continue;
    const dScroll = b.scrollTop - a.scrollTop;
    const dev = rowMove - dScroll;
    const aboveDelta = b.offsetTop - a.offsetTop;
    const cause: Cause =
      Math.abs(aboveDelta) > ABOVE_EPS ? "reflow-driven" : "tracking-failure";
    reversals.push({
      i,
      rowMove,
      dScroll,
      dev,
      aboveDelta,
      cause,
      rowId: b.rowId,
    });
  }

  const maxReversalPx =
    reversals.length === 0
      ? 0
      : Math.max(...reversals.map((r) => Math.abs(r.rowMove)));
  const stillFrame = reversals.filter((r) => Math.abs(r.dScroll) < REVERSAL_PX);
  const byCause = (c: Cause) => reversals.filter((r) => r.cause === c).length;

  // A5 CENSUS (impl-agnostic form). Dawn's contract A5 — "the corrector doesn't
  // fight live momentum" — was written around OUR scroll-writes: near-zero
  // mid-momentum correction writes = proof of no fight. "Writes" is a
  // `useAnchoredScroll` internal; a virtualizer has none. The observable that
  // survives the impl swap is the FELT consequence a write would cause: a
  // backward row jerk WHILE the flick is fast. So the portable A5 census counts
  // TRACKING-FAILURE reversals occurring inside a momentum window (|dScroll| at
  // or above a fast-flick threshold). Near-zero here = the impl injects no
  // backward motion mid-momentum = doesn't fight — provable from DOM alone, not
  // from trackpad vibes (the subjective floor #1662 fell through). Reflow-driven
  // reversals are excluded: those are the A3 reflow class, a different gate; A5
  // is specifically about anchoring fighting the wheel with no layout cause.
  const MOMENTUM_PX = 8; // |dScroll| per frame that reads as live flick, not a still frame
  const midMomentumFight = reversals.filter(
    (r) => Math.abs(r.dScroll) >= MOMENTUM_PX && r.cause === "tracking-failure",
  );
  // POST-MOMENTUM BITE (#1662 coalesced-desync signature). Eva's read of the
  // vacuous mid-momentum cell: the WebKit compositor desync does not render
  // DURING the momentum window — it lands on the coalesced STILL frame one or
  // two frames AFTER momentum, when scrollTop reports 0 delta but the row snaps
  // back. So the discriminating signature is a still-frame TRACKING-FAILURE
  // reversal that TRAILS a recent momentum frame. Unlike the raw mid-momentum
  // filter (which our known-bad impl scores 0 on, making it vacuous), this
  // captures the frames where the bite actually appears — so it is non-zero on
  // ours and a candidate that eliminates the stale-window write drops it.
  const POST_MOMENTUM_FRAMES = 3; // bite lands within this many frames of momentum end
  const hadRecentMomentum = (frameIndex: number): boolean => {
    for (
      let j = frameIndex - 1;
      j >= Math.max(1, frameIndex - POST_MOMENTUM_FRAMES);
      j -= 1
    ) {
      const d = Math.abs(frames[j].scrollTop - frames[j - 1].scrollTop);
      if (d >= MOMENTUM_PX) return true;
    }
    return false;
  };
  const postMomentumBite = reversals.filter(
    (r) =>
      r.cause === "tracking-failure" &&
      Math.abs(r.dScroll) < REVERSAL_PX &&
      hadRecentMomentum(r.i),
  );
  // Restricted max px — Dawn's co-gate (thread evt 04a7e28b response). The
  // discriminator is bite COUNT *and* bite MAX-PX both down, not count alone: a
  // candidate could drop bite count while leaving the survivors larger (fewer,
  // more violent snaps — worse felt), and count-only would score that a win.
  // The GLOBAL maxReversalPx above spans ALL reversals; this is the max snap
  // WITHIN the post-momentum-bite set, so the co-gate reads the felt severity of
  // exactly the frames the discriminator counts.
  const postMomentumBiteMaxPx =
    postMomentumBite.length === 0
      ? 0
      : Math.max(...postMomentumBite.map((r) => Math.abs(r.rowMove)));
  const momentumFrames = (() => {
    let n = 0;
    for (let i = 1; i < frames.length; i += 1) {
      const a = frames[i - 1];
      const b = frames[i];
      if (Math.abs(b.scrollTop - a.scrollTop) >= MOMENTUM_PX) n += 1;
    }
    return n;
  })();

  /* eslint-disable no-console */
  console.log("\n=== PORTABLE UPSCROLL CLASSIFY (impl-agnostic) ===");
  console.log(`engine:                 ${browserName}`);
  console.log(`frames sampled:         ${frames.length}`);
  console.log(`frame-pairs scored:     ${scored}`);
  console.log(`re-anchor/skip frames:  ${reanchors}`);
  console.log(`reversal frames:        ${reversals.length}`);
  console.log(`  of which still-frame: ${stillFrame.length}`);
  console.log(`max reversal px:        ${maxReversalPx.toFixed(1)}`);
  console.log("--- reversal cause (pure DOM: offsetTop delta) ---");
  console.log(
    `  REFLOW-DRIVEN (content above moved):     ${byCause("reflow-driven")}`,
  );
  console.log(
    `  TRACKING-FAILURE (anchor still, reversed): ${byCause("tracking-failure")}`,
  );
  console.log("--- A5 census: does the impl fight live momentum? ---");
  console.log(
    `  momentum frames (|dScroll|>=${MOMENTUM_PX}px):     ${momentumFrames}`,
  );
  console.log(
    `  mid-momentum tracking-failure jerks:  ${midMomentumFight.length} (vacuous on known-bad main — see post-momentum)`,
  );
  console.log(
    `  POST-momentum bite (still-frame ≤${POST_MOMENTUM_FRAMES}f after flick): ${postMomentumBite.length} (#1662 signature — the real discriminator)`,
  );
  console.log(
    `    └ bite-set max px (co-gate w/ count): ${postMomentumBiteMaxPx.toFixed(1)}`,
  );
  for (const r of reversals
    .slice()
    .sort((x, y) => x.rowMove - y.rowMove)
    .slice(0, 12)) {
    console.log(
      `  frame ${r.i} rowMove=${r.rowMove.toFixed(1)} dScroll=${r.dScroll.toFixed(1)} dev=${r.dev.toFixed(1)} aboveΔ=${r.aboveDelta.toFixed(1)} cause=${r.cause} row=${r.rowId}`,
    );
  }
  console.log("==================================================\n");
  /* eslint-enable no-console */

  // Sanity: the actuation produced a scored upscroll. This is the ONLY hard
  // assertion — the fixture characterizes for comparison, it does not gate a
  // ceiling (a candidate is judged by the printed columns vs ours, plus Tyler's
  // real-trackpad pass).
  expect(scored).toBeGreaterThan(50);
});
