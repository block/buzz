import { test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

/**
 * SLOW-TRACKPAD MISMATCH PROBE (diagnostic, non-gating) — Tyler's ask 2026-07-08:
 * "Try playwright trackpad style scrolling slowly and find the movement
 * mismatches and fix them."
 *
 * Actuation: SMALL wheel deltas (12px) at slow cadence (~32ms), the shape of a
 * slow deliberate trackpad drag — NOT the 220px settled notches of the jitter
 * gate. Slow scrolling maximizes the ratio of realization delta to scroll
 * delta, which is exactly where per-notch residuals feel worst.
 *
 * Sampling: an in-page rAF loop records, EVERY FRAME, the tracked reading
 * row's rect.top, scrollTop, and mounted-row count while wheel events arrive
 * asynchronously. No settling — we want the raw trajectory including any
 * one-frame flash before a deferred correction lands.
 *
 * Mismatch signal (diagnostic only — NOT a gate metric). The felt event is the
 * reading row's own screen motion, pure rect.top (per Quinn's W4 framing —
 * no scrollTop, no input reference in the FLAG condition). Under slow 12px
 * wheel input the row's per-frame trajectory is a 0/+12 staircase (Sami's
 * quantization finding), so we flag frames whose rowMove falls OUTSIDE the
 * staircase envelope:
 *   - REVERSAL: rowMove <= -3 (row moves against the scroll direction)
 *   - SHOVE:    rowMove >= 2*WHEEL_DELTA + 3 (row jumps more than two
 *               coalesced wheel events could produce)
 * For each flagged frame we also record e = rowMove + dScroll — which by the
 * rect.top identity equals the raw reflow-above-row that frame (comp-
 * INVARIANT: nonzero even when perfectly compensated). e is CONTEXT ONLY:
 * it distinguishes "reflow reached the row uncompensated" (rowMove tracks e)
 * from "correction landed a frame late" (a +/- rowMove pair, e on the first).
 * Frames where the tracked row was re-picked or a prepend landed are marked,
 * not scored.
 */

const WHEEL_DELTA = 12; // px per wheel event — slow deliberate trackpad
const WHEEL_PERIOD_MS = 32; // cadence
const DURATION_MS = 30_000; // total actuation time
const SAFE_MARGIN = 100;
const E_NOTABLE = 3; // px of unexplained row motion worth logging

type Frame = {
  t: number;
  top: number | null; // tracked row rect.top (null = row left DOM)
  scrollTop: number;
  mounted: number;
  rowId: string | null; // id of tracked row this frame
};

test("PROBE: slow-trackpad movement mismatches (diagnostic)", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
  );
  await page.getByTestId("channel-jitter-corpus").click();
  const timeline = page.getByTestId("message-timeline");
  await page.waitForFunction(() => {
    const el = document.querySelector(
      '[data-testid="message-timeline"]',
    ) as HTMLDivElement | null;
    return !!el && el.scrollHeight > el.clientHeight + 1000;
  });

  // Pin to bottom; force overflow-anchor none (mirror shipped WKWebView).
  await timeline.evaluate((element) => {
    const el = element as HTMLDivElement;
    el.style.overflowAnchor = "none";
    el.scrollTop = el.scrollHeight;
    el.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  await page.waitForTimeout(200);
  await timeline.hover();

  // Start the in-page per-frame sampler. It re-picks the tracked row when the
  // current one leaves the safe band (marking the frame), and records every
  // rAF tick until told to stop.
  await timeline.evaluate((element, margin: number) => {
    const el = element as HTMLDivElement;
    const w = window as unknown as {
      __PROBE__: { frames: Frame[]; stop: boolean };
    };
    type Frame = {
      t: number;
      top: number | null;
      scrollTop: number;
      mounted: number;
      rowId: string | null;
    };
    w.__PROBE__ = { frames: [], stop: false };
    let trackedId: string | null = null;
    const pick = (): string | null => {
      const box = el.getBoundingClientRect();
      const rows = el.querySelectorAll<HTMLElement>("[data-message-id]");
      for (const row of rows) {
        const rect = row.getBoundingClientRect();
        if (rect.top > box.top + margin && rect.bottom < box.bottom - margin) {
          return row.dataset.messageId ?? null;
        }
      }
      return null;
    };
    const tick = (t: number) => {
      if (w.__PROBE__.stop) return;
      const mounted = el.querySelectorAll("[data-message-id]").length;
      let top: number | null = null;
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
        }
      }
      if (top === null) {
        trackedId = pick(); // re-pick; this frame is a re-anchor marker
      }
      w.__PROBE__.frames.push({
        t,
        top,
        scrollTop: el.scrollTop,
        mounted,
        rowId: trackedId,
      });
      requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  }, SAFE_MARGIN);

  // Slow trackpad actuation: small deltas, steady cadence, no settling.
  const started = Date.now();
  while (Date.now() - started < DURATION_MS) {
    await page.mouse.wheel(0, -WHEEL_DELTA);
    await page.waitForTimeout(WHEEL_PERIOD_MS);
  }

  const frames: Frame[] = await timeline.evaluate((_el) => {
    const w = window as unknown as {
      __PROBE__: { frames: Frame[]; stop: boolean };
    };
    type Frame = {
      t: number;
      top: number | null;
      scrollTop: number;
      mounted: number;
      rowId: string | null;
    };
    w.__PROBE__.stop = true;
    return w.__PROBE__.frames;
  });

  // Analysis: per-frame rowMove (pure rect.top), scored only across
  // consecutive frames tracking the SAME row with valid tops. Flag frames
  // whose rowMove escapes the slow-wheel staircase envelope.
  const REVERSAL_PX = -E_NOTABLE; // row moved against the scroll
  const SHOVE_PX = 2 * WHEEL_DELTA + E_NOTABLE; // > two coalesced notches
  type Ev = {
    i: number;
    t: number;
    kind: "REVERSAL" | "SHOVE";
    rowMove: number;
    e: number; // rowMove + dScroll = raw reflow-above-row (context only)
    dScroll: number;
    dMounted: number;
    rowId: string | null;
  };
  const events: Ev[] = [];
  const rowMoves: number[] = [];
  let scored = 0;
  let reanchors = 0;
  let prepends = 0;
  for (let i = 1; i < frames.length; i += 1) {
    const a = frames[i - 1];
    const b = frames[i];
    const dMounted = b.mounted - a.mounted;
    if (dMounted > 0) prepends += 1;
    if (
      a.top === null ||
      b.top === null ||
      a.rowId === null ||
      a.rowId !== b.rowId
    ) {
      reanchors += 1;
      continue;
    }
    const rowMove = b.top - a.top;
    const dScroll = b.scrollTop - a.scrollTop;
    const e = rowMove + dScroll;
    scored += 1;
    rowMoves.push(rowMove);
    const kind =
      rowMove <= REVERSAL_PX
        ? ("REVERSAL" as const)
        : rowMove >= SHOVE_PX
          ? ("SHOVE" as const)
          : null;
    if (kind) {
      events.push({
        i,
        t: b.t,
        kind,
        rowMove,
        e,
        dScroll,
        dMounted,
        rowId: b.rowId,
      });
    }
  }

  // Flash pairing: a shove/reversal cancelled by an opposite move within 3
  // frames is a one-frame flash (deferred correction); unpaired = felt lurch.
  const flashes: Array<[Ev, Ev]> = [];
  const lurches: Ev[] = [];
  const used = new Set<number>();
  for (let k = 0; k < events.length; k += 1) {
    if (used.has(k)) continue;
    const ev = events[k];
    let paired = false;
    for (let m = k + 1; m < events.length && events[m].i - ev.i <= 3; m += 1) {
      if (used.has(m)) continue;
      // Opposite-signed rowMove of comparable magnitude (allowing the +12
      // staircase riding on top of the correction).
      const cancel = events[m].rowMove + ev.rowMove;
      if (Math.abs(cancel) <= Math.abs(ev.rowMove) * 0.4 + WHEEL_DELTA) {
        flashes.push([ev, events[m]]);
        used.add(k);
        used.add(m);
        paired = true;
        break;
      }
    }
    if (!paired) {
      lurches.push(ev);
      used.add(k);
    }
  }
  // Net signed drift of the row across all scored frames, minus the expected
  // downward staircase — sustained same-sign residue = skip-forever shape.
  const totalRowMove = rowMoves.reduce((acc, m) => acc + m, 0);

  /* eslint-disable no-console */
  console.log("\n=== SLOW-TRACKPAD MISMATCH PROBE (diagnostic) ===");
  console.log(
    `wheel: ${WHEEL_DELTA}px every ${WHEEL_PERIOD_MS}ms for ${DURATION_MS / 1000}s`,
  );
  console.log(`frames sampled:            ${frames.length}`);
  console.log(`frame-pairs scored:        ${scored}`);
  console.log(`re-anchor/skip frames:     ${reanchors}`);
  console.log(`prepend commits observed:  ${prepends}`);
  console.log(
    `envelope escapes (reversal<=${REVERSAL_PX} or shove>=${SHOVE_PX}): ${events.length}  ` +
      `(flash-pairs: ${flashes.length}, unpaired lurches: ${lurches.length})`,
  );
  console.log(
    `total tracked rowMove:     ${totalRowMove.toFixed(1)}px over ${scored} frames`,
  );
  const fmt = (ev: Ev) =>
    `  frame ${ev.i} t=${ev.t.toFixed(0)} ${ev.kind} rowMove=${ev.rowMove.toFixed(1)} ` +
    `(e=${ev.e.toFixed(1)}, dScroll=${ev.dScroll.toFixed(1)}, ` +
    `dMounted=${ev.dMounted}, row=${ev.rowId})`;
  console.log("--- worst unpaired lurches (top 12 by |rowMove|) ---");
  for (const ev of lurches
    .slice()
    .sort((x, y) => Math.abs(y.rowMove) - Math.abs(x.rowMove))
    .slice(0, 12)) {
    console.log(fmt(ev));
  }
  console.log("--- worst flash pairs (top 6) ---");
  for (const [x, y] of flashes
    .slice()
    .sort((p, q) => Math.abs(q[0].rowMove) - Math.abs(p[0].rowMove))
    .slice(0, 6)) {
    console.log(fmt(x));
    console.log(fmt(y));
  }
  console.log("=================================================\n");
  /* eslint-enable no-console */
});
