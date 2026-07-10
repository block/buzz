import { expect, test } from "@playwright/test";
import { decode } from "nostr-tools/nip19";
import { getPublicKey } from "nostr-tools/pure";

import { installRelayBridge } from "../helpers/bridge";

/**
 * LIVE TRACKPAD-MOMENTUM UPSCROLL PROBE — Tyler: "the jumping is bad even on
 * PR 1650" with a trackpad. The prior probes settled 1-2 RAFs after every
 * step; a trackpad does not. This probe replicates macOS trackpad swipes:
 * trusted CDP mouseWheel events at ~8ms spacing — finger ramp then an
 * exponentially-decaying momentum tail — fired CONTINUOUSLY through real
 * fetchOlder commits in #buzz-bugs against live staging. No settling.
 *
 * Measurement is a per-RAF in-page sampler, independent of input timing:
 * every frame it records scrollTop + the tracked center row's rect.top +
 * mounted count + fetch count. For a solid page,
 *   rowMove(frame) = rect.top delta = scrollTop_before - scrollTop_after.
 * Per-frame deviation = rowMove - appliedScroll. Nonzero = content shifted
 * under the viewport that the input didn't ask for — the felt jump —
 * regardless of whether it came from CV realization, a losing compensation
 * race, or a prepend anchor miss.
 *
 * Run: same env as upscroll-1px-live.perf.ts.
 */

const RELAY_HTTP =
  process.env.BUZZ_E2E_RELAY_URL ?? "https://sprout-oss.stage.blox.sqprod.co";
const NSEC = process.env.BUZZ_PERF_NSEC ?? "";
const COMMUNITY_HOST = process.env.BUZZ_COMMUNITY_HOST ?? "";
const TARGET_CHANNEL = process.env.BUZZ_PERF_CHANNEL ?? "buzz-bugs";
const SWIPES = Number(process.env.BUZZ_PERF_SWIPES ?? 30);
const SAFE_MARGIN = 60;

const IDENTITY_OVERRIDE_KEY = "buzz:e2e-identity-override.v1";
const ONBOARDING_PREFIX = "buzz-onboarding-complete.v1:";
const WELCOME_PREFIX = "buzz-welcome-channel-ensured.v2:";
const REAL_CHROME_UA =
  "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36";

test.use({ userAgent: REAL_CHROME_UA });

function deriveIdentity(nsec: string) {
  const decoded = decode(nsec.trim());
  if (decoded.type !== "nsec") throw new Error("BUZZ_PERF_NSEC is not an nsec");
  const skBytes = decoded.data as Uint8Array;
  return {
    privateKey: Buffer.from(skBytes).toString("hex"),
    pubkey: getPublicKey(skBytes),
    username: "perf-eva",
  };
}

// One macOS-ish swipe: finger ramp (accelerating) + momentum tail (exp decay).
function swipeDeltas(): number[] {
  const deltas: number[] = [];
  // finger: ~12 events ramping 4 -> 36 px
  for (let i = 0; i < 12; i++) deltas.push(4 + Math.round((32 * i) / 11));
  // momentum: decay from 36 by 0.94/event until < 1
  let v = 36;
  while (v >= 1) {
    deltas.push(Math.round(v));
    v *= 0.94;
  }
  return deltas; // ~68 events, ~1500px total
}

type Frame = {
  t: number;
  scrollTop: number;
  rowId: string | null;
  rowTop: number | null;
  mounted: number;
  fetch: number;
};

test("MEASURE: live trackpad-momentum upscroll profile", async ({ page }) => {
  test.setTimeout(900_000);
  if (!NSEC) throw new Error("Set BUZZ_PERF_NSEC to a real member nsec");
  const identity = deriveIdentity(NSEC);

  await installRelayBridge(page, "tyler");
  const wsUrl = RELAY_HTTP.replace(/^http/, "ws");
  await page.addInitScript(
    ({ ident, onboardingPrefix, welcomePrefix, relayUrl, overrideKey }) => {
      window.localStorage.setItem(overrideKey, JSON.stringify(ident));
      window.localStorage.setItem(`${onboardingPrefix}${ident.pubkey}`, "true");
      window.localStorage.setItem(
        `${welcomePrefix}${encodeURIComponent(relayUrl)}:${ident.pubkey}`,
        "true",
      );
      const w = window as unknown as { __BUZZ_E2E__?: Record<string, unknown> };
      w.__BUZZ_E2E__ = { ...(w.__BUZZ_E2E__ ?? {}), identity: ident };
    },
    {
      ident: identity,
      onboardingPrefix: ONBOARDING_PREFIX,
      welcomePrefix: WELCOME_PREFIX,
      relayUrl: wsUrl,
      overrideKey: IDENTITY_OVERRIDE_KEY,
    },
  );

  const relayHost = new URL(RELAY_HTTP).host;
  await page.route(
    (url) => url.host === relayHost,
    async (route) => {
      const req = route.request();
      const fwd = { ...req.headers(), "user-agent": REAL_CHROME_UA };
      delete fwd["sec-ch-ua"];
      delete fwd["sec-ch-ua-mobile"];
      delete fwd["sec-ch-ua-platform"];
      if (COMMUNITY_HOST) fwd.host = COMMUNITY_HOST;
      let resp: Awaited<ReturnType<typeof route.fetch>> | null = null;
      for (let attempt = 0; attempt < 5; attempt++) {
        try {
          resp = await route.fetch({ headers: fwd });
          break;
        } catch (err) {
          if (attempt === 4) throw err;
          await new Promise((r) => setTimeout(r, 1500));
        }
      }
      if (!resp) throw new Error("unreachable");
      const headers = { ...resp.headers() };
      headers["access-control-allow-origin"] = "*";
      headers["access-control-allow-headers"] = "*";
      headers["access-control-allow-methods"] = "*";
      await route.fulfill({ response: resp, headers, body: await resp.body() });
    },
  );

  await page.goto("/");
  await page.getByTestId("app-sidebar").waitFor({ state: "visible" });
  const chan = page.getByTestId(`channel-${TARGET_CHANNEL}`).first();
  await chan.waitFor({ state: "visible", timeout: 45_000 });
  await chan.click();
  await page
    .locator('[data-testid="message-timeline"] [data-message-id]')
    .first()
    .waitFor({ state: "visible", timeout: 30_000 });
  await page.waitForTimeout(3000);

  const timeline = page.getByTestId("message-timeline");
  // WKWebView mirror.
  await timeline.evaluate((el) => {
    (el as HTMLElement).style.overflowAnchor = "none";
  });
  await timeline.evaluate((element) => {
    const el = element as HTMLDivElement;
    el.scrollTop = el.scrollHeight;
    el.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  await page.waitForTimeout(500);

  // ---- Per-RAF sampler, in page, independent of input cadence ----
  await timeline.evaluate((element, margin: number) => {
    const el = element as HTMLDivElement;
    const store = window as unknown as {
      __FRAMES__: Frame[];
      __SAMPLER_STOP__?: boolean;
      __CHANNEL_WINDOW_FETCH_COUNT__?: number;
    };
    type Frame = {
      t: number;
      scrollTop: number;
      rowId: string | null;
      rowTop: number | null;
      mounted: number;
      fetch: number;
    };
    store.__FRAMES__ = [];
    let trackedId: string | null = null;
    const pick = (): string | null => {
      const box = el.getBoundingClientRect();
      const mid = box.top + box.height / 2;
      let best: { id: string; d: number } | null = null;
      for (const row of el.querySelectorAll<HTMLElement>("[data-message-id]")) {
        const r = row.getBoundingClientRect();
        if (r.top <= box.top + margin || r.bottom >= box.bottom - margin)
          continue;
        const d = Math.abs((r.top + r.bottom) / 2 - mid);
        if (!best || d < best.d) best = { id: row.dataset.messageId ?? "", d };
      }
      return best?.id || null;
    };
    const loop = () => {
      if (store.__SAMPLER_STOP__) return;
      const box = el.getBoundingClientRect();
      let rowTop: number | null = null;
      if (trackedId) {
        const row = el.querySelector<HTMLElement>(
          `[data-message-id="${CSS.escape(trackedId)}"]`,
        );
        if (row) {
          const r = row.getBoundingClientRect();
          if (r.top > box.top + margin && r.bottom < box.bottom - margin)
            rowTop = r.top;
        }
      }
      if (rowTop === null) {
        trackedId = pick();
        if (trackedId) {
          const r = el
            .querySelector<HTMLElement>(
              `[data-message-id="${CSS.escape(trackedId)}"]`,
            )
            ?.getBoundingClientRect();
          rowTop = r ? r.top : null;
        }
      }
      store.__FRAMES__.push({
        t: performance.now(),
        scrollTop: el.scrollTop,
        rowId: trackedId,
        rowTop,
        mounted: el.querySelectorAll("[data-message-id]").length,
        fetch: store.__CHANNEL_WINDOW_FETCH_COUNT__ ?? 0,
      });
      requestAnimationFrame(loop);
    };
    requestAnimationFrame(loop);
  }, SAFE_MARGIN);

  // ---- Trusted trackpad-like wheel input via CDP ----
  const box = await timeline.boundingBox();
  if (!box) throw new Error("no timeline box");
  const cx = box.x + box.width / 2;
  const cy = box.y + box.height / 2;
  const isChromium =
    page.context().browser()?.browserType().name() === "chromium";
  const cdp = isChromium ? await page.context().newCDPSession(page) : null;
  await page.mouse.move(cx, cy);

  for (let s = 0; s < SWIPES; s++) {
    const deltas = swipeDeltas();
    for (const d of deltas) {
      if (cdp) {
        await cdp.send("Input.dispatchMouseEvent", {
          type: "mouseWheel",
          x: cx,
          y: cy,
          deltaX: 0,
          deltaY: -d,
          pointerType: "mouse",
        });
      } else {
        await page.mouse.wheel(0, -d);
      }
      await new Promise((r) => setTimeout(r, 8));
    }
    // brief finger-off pause between swipes, like a real gesture chain
    await page.waitForTimeout(120);
    const at = await timeline.evaluate(
      (el) => (el as HTMLDivElement).scrollTop,
    );
    if (at <= 0) {
      // at the wall — give the prepend a chance to land, keep swiping after
      await page.waitForTimeout(2500);
    }
    if (s % 5 === 4)
      console.log(
        `[swipe ${s + 1}/${SWIPES}] scrollTop=${at.toFixed(0)} frames=${await page.evaluate(() => (window as unknown as { __FRAMES__: unknown[] }).__FRAMES__.length)}`,
      );
  }

  await page.evaluate(() => {
    (window as unknown as { __SAMPLER_STOP__?: boolean }).__SAMPLER_STOP__ =
      true;
  });
  const frames = (await page.evaluate(
    () => (window as unknown as { __FRAMES__: Frame[] }).__FRAMES__,
  )) as Frame[];

  // ---- Analysis: per-frame deviation of row motion vs applied scroll ----
  type Dev = {
    i: number;
    dev: number;
    applied: number;
    rowMove: number;
    dt: number;
    mountedGrew: boolean;
    fetch: number;
    scrollTop: number;
  };
  const devs: Dev[] = [];
  for (let i = 1; i < frames.length; i++) {
    const a = frames[i - 1];
    const b = frames[i];
    if (!a.rowId || a.rowId !== b.rowId) continue; // re-pick boundary
    if (a.rowTop === null || b.rowTop === null) continue;
    const applied = a.scrollTop - b.scrollTop;
    const rowMove = b.rowTop - a.rowTop;
    devs.push({
      i,
      dev: rowMove - applied,
      applied,
      rowMove,
      dt: b.t - a.t,
      mountedGrew: b.mounted > a.mounted,
      fetch: b.fetch,
      scrollTop: b.scrollTop,
    });
  }

  const scored = devs.filter((d) => !d.mountedGrew); // prepend-commit frames reported separately
  const commits = devs.filter((d) => d.mountedGrew);
  const jumps = scored
    .filter((d) => Math.abs(d.dev) > 2)
    .sort((x, y) => Math.abs(y.dev) - Math.abs(x.dev));

  const hist = new Map<number, number>();
  for (const d of scored) {
    const bucket =
      Math.abs(d.dev) <= 1
        ? 0
        : Math.sign(d.dev) * 2 ** Math.ceil(Math.log2(Math.abs(d.dev)));
    hist.set(bucket, (hist.get(bucket) ?? 0) + 1);
  }

  console.log(
    `\n=== LIVE TRACKPAD PROFILE: #${TARGET_CHANNEL} engine=${page.context().browser()?.browserType().name()} ===`,
  );
  console.log(
    `frames=${frames.length} scoredFramePairs=${scored.length} swipes=${SWIPES} fetchPages=${frames[frames.length - 1]?.fetch ?? 0} finalMounted=${frames[frames.length - 1]?.mounted ?? 0}`,
  );
  console.log(
    "\n--- per-frame deviation histogram (0 = tracked row moved exactly what input asked; buckets are +/- powers of 2 px) ---",
  );
  for (const [k, v] of [...hist.entries()].sort((a, b) => a[0] - b[0]))
    console.log(`  ${String(k).padStart(6)} : ${v}`);
  console.log(
    `\nsmooth frames (|dev|<=1px): ${scored.filter((d) => Math.abs(d.dev) <= 1).length}/${scored.length} = ${((100 * scored.filter((d) => Math.abs(d.dev) <= 1).length) / Math.max(1, scored.length)).toFixed(2)}%`,
  );
  console.log(`\n--- jump frames |dev|>2px: ${jumps.length} ---`);
  for (const d of jumps.slice(0, 50))
    console.log(
      `  frame=${d.i} dev=${d.dev.toFixed(1)}px applied=${d.applied.toFixed(1)} rowMove=${d.rowMove.toFixed(1)} dt=${d.dt.toFixed(1)}ms fetch=${d.fetch} scrollTop=${d.scrollTop.toFixed(0)}`,
    );
  console.log(`\n--- prepend-commit frames (${commits.length}) ---`);
  for (const d of commits)
    console.log(
      `  frame=${d.i} dev=${d.dev.toFixed(1)}px applied=${d.applied.toFixed(1)} rowMove=${d.rowMove.toFixed(1)} dt=${d.dt.toFixed(1)}ms fetch=${d.fetch} scrollTop=${d.scrollTop.toFixed(0)}`,
    );

  // long frames = jank of a different kind (main-thread stalls)
  const longFrames = devs.filter((d) => d.dt > 34);
  console.log(
    `\n--- long frames (>34ms, i.e. dropped at least one 60Hz frame): ${longFrames.length} ---`,
  );
  for (const d of longFrames.slice(0, 25))
    console.log(
      `  frame=${d.i} dt=${d.dt.toFixed(0)}ms dev=${d.dev.toFixed(1)} applied=${d.applied.toFixed(1)} fetch=${d.fetch} scrollTop=${d.scrollTop.toFixed(0)}`,
    );
  console.log("==============================================\n");

  expect(frames.length).toBeGreaterThan(500);
});
