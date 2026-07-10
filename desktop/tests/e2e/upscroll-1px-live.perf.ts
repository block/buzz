import { expect, test } from "@playwright/test";
import { decode } from "nostr-tools/nip19";
import { getPublicKey } from "nostr-tools/pure";

import { installRelayBridge } from "../helpers/bridge";

/**
 * LIVE 1-PIXEL UPSCROLL PROBE — Tyler's ask (2026-07-08):
 * run the GUI against the LIVE staging relay, scroll up 1px at a time for
 * many thousands of pixels (crossing multiple real fetchOlder scrollback
 * loads in #buzz-bugs), and record how many pixels of VISIBLE MOVEMENT occur
 * per 1px of input.
 *
 * Method: same live-relay harness as scrollback-buzzbugs.perf.ts (identity
 * override, port-forward + Host rewrite, ACAO injection). Actuation is a
 * synchronous `scrollTop -= 1` + scroll event per step with one RAF settle,
 * batched in-page (800 steps/evaluate) so 15k+ steps are feasible. Per step
 * we record:
 *   move       = tracked center row's rect.top delta   (the felt movement)
 *   appliedTop = scrollTop_before - scrollTop_after    (what the scroller did)
 *   mounted    = [data-message-id] count               (prepend detection)
 *   fetch      = __CHANNEL_WINDOW_FETCH_COUNT__        (fetchOlder activity)
 * Smooth = move === +1 every step. Jank = |move| >> 1 (realization lurch,
 * anchor jump, prepend re-anchor).
 *
 * WKWebView mirror: `overflow-anchor: none` forced on the scroller (the
 * shipped engine has no scroll anchoring); the production computed value is
 * logged first, un-forced, for the anchor-contract record.
 *
 * Run:
 *   kubectl --context bke-coder-stage -n sprout port-forward svc/sprout-relay 13000:3000 &
 *   BUZZ_E2E_RELAY_URL=http://127.0.0.1:13000 \
 *   BUZZ_COMMUNITY_HOST=sprout-oss.stage.blox.sqprod.co \
 *   BUZZ_PERF_NSEC=nsec1... \
 *   npx playwright test --config=playwright.perf.config.ts upscroll-1px-live
 */

const RELAY_HTTP =
  process.env.BUZZ_E2E_RELAY_URL ?? "https://sprout-oss.stage.blox.sqprod.co";
const NSEC = process.env.BUZZ_PERF_NSEC ?? "";
const COMMUNITY_HOST = process.env.BUZZ_COMMUNITY_HOST ?? "";
const TARGET_CHANNEL = process.env.BUZZ_PERF_CHANNEL ?? "buzz-bugs";
// Stop when BOTH: at least this many fetchOlder pages landed AND this many
// pixels walked — or the step cap / top of history, whichever first.
const MIN_FETCHES = Number(process.env.BUZZ_PERF_MIN_FETCHES ?? 3);
const MIN_PX = Number(process.env.BUZZ_PERF_MIN_PX ?? 8000);
const MAX_STEPS = Number(process.env.BUZZ_PERF_MAX_STEPS ?? 20000);
const BATCH = 800;
const SAFE_MARGIN = 60;
// Pixels per input step (1 = original probe; 100+ = Tyler's big-notch repro).
const STEP_PX = Number(process.env.BUZZ_PERF_STEP_PX ?? 1);

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

type Step = {
  i: number;
  move: number | null; // tracked row rect.top delta; null = row lost this step
  appliedTop: number;
  mounted: number;
  fetch: number;
  scrollTop: number;
  repick: boolean; // tracked row re-picked before this step
};

test("MEASURE: live 1px upscroll movement profile", async ({ page }) => {
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
      // The port-forward can drop mid-run; retry a few times rather than
      // aborting a 15k-step walk on one transient ECONNREFUSED.
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
      const body = req.postData() ?? "";
      if (body.includes("before_id") || body.includes("until")) {
        console.log(
          `[net] ${new URL(req.url()).pathname} ${resp.status()} ${(await resp.body()).byteLength}b body=${body.slice(0, 140)}`,
        );
      }
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
  await page.waitForTimeout(3000); // let the open burst settle

  const timeline = page.getByTestId("message-timeline");

  // Production anchor contract, read BEFORE forcing anything.
  const anchor = await timeline.evaluate((el) => ({
    supports: CSS.supports("overflow-anchor", "none"),
    computed: getComputedStyle(el as HTMLElement).overflowAnchor ?? "(none)",
  }));
  console.log(
    `[anchor] CSS.supports=${anchor.supports} production computed=${anchor.computed}`,
  );
  // WKWebView mirror: no engine-side anchoring during the walk.
  await timeline.evaluate((el) => {
    (el as HTMLElement).style.overflowAnchor = "none";
  });

  // Pin to true bottom.
  await timeline.evaluate((element) => {
    const el = element as HTMLDivElement;
    el.scrollTop = el.scrollHeight;
    el.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  await page.waitForTimeout(500);

  const all: Step[] = [];
  const prependCommits: Array<{
    step: number;
    mountedFrom: number;
    mountedTo: number;
    scrollTopTo: number;
    feltJump: number | null;
  }> = [];
  let ended = "cap";
  let stepIndex = 0;

  while (stepIndex < MAX_STEPS) {
    const batch = await timeline.evaluate(
      async (
        element,
        args: { n: number; margin: number; base: number; step: number },
      ) => {
        const el = element as HTMLDivElement;
        const probe = window as unknown as {
          __CHANNEL_WINDOW_FETCH_COUNT__?: number;
        };
        const out: Array<{
          i: number;
          move: number | null;
          appliedTop: number;
          mounted: number;
          fetch: number;
          scrollTop: number;
          repick: boolean;
        }> = [];
        let end = "";
        let trackedId: string | null = null;

        const pick = (): { id: string; top: number } | null => {
          const box = el.getBoundingClientRect();
          const mid = box.top + box.height / 2;
          let best: { id: string; top: number; d: number } | null = null;
          for (const row of el.querySelectorAll<HTMLElement>(
            "[data-message-id]",
          )) {
            const r = row.getBoundingClientRect();
            if (
              r.top <= box.top + args.margin ||
              r.bottom >= box.bottom - args.margin
            )
              continue;
            const d = Math.abs((r.top + r.bottom) / 2 - mid);
            if (!best || d < best.d)
              best = { id: row.dataset.messageId ?? "", top: r.top, d };
          }
          return best && best.id ? { id: best.id, top: best.top } : null;
        };

        const locate = (id: string): number | null => {
          const box = el.getBoundingClientRect();
          const row = el.querySelector<HTMLElement>(
            `[data-message-id="${CSS.escape(id)}"]`,
          );
          if (!row) return null;
          const r = row.getBoundingClientRect();
          if (
            r.top <= box.top + args.margin ||
            r.bottom >= box.bottom - args.margin
          )
            return null; // left safe band
          return r.top;
        };

        for (let i = 0; i < args.n; i++) {
          if (el.scrollTop <= 0) {
            end = "top";
            break;
          }
          let repick = false;
          let beforeTop = trackedId ? locate(trackedId) : null;
          if (beforeTop === null) {
            const p = pick();
            repick = true;
            if (!p) {
              trackedId = null;
            } else {
              trackedId = p.id;
              beforeTop = p.top;
            }
          }
          const beforeScroll = el.scrollTop;

          el.scrollTop = Math.max(0, el.scrollTop - args.step);
          el.dispatchEvent(new Event("scroll", { bubbles: true }));
          await new Promise<void>((r) => requestAnimationFrame(() => r()));

          const afterTop = trackedId ? locate(trackedId) : null;
          out.push({
            i: args.base + i,
            move:
              beforeTop !== null && afterTop !== null
                ? afterTop - beforeTop
                : null,
            appliedTop: beforeScroll - el.scrollTop,
            mounted: el.querySelectorAll("[data-message-id]").length,
            fetch: probe.__CHANNEL_WINDOW_FETCH_COUNT__ ?? 0,
            scrollTop: el.scrollTop,
            repick,
          });
        }
        return { out, end };
      },
      { n: BATCH, margin: SAFE_MARGIN, base: stepIndex, step: STEP_PX },
    );

    all.push(...batch.out);
    stepIndex += batch.out.length;
    const fetches = all.length ? all[all.length - 1].fetch : 0;
    const walked = all.reduce((s, x) => s + Math.max(0, x.appliedTop), 0);
    console.log(
      `[progress] steps=${stepIndex} walkedPx=${walked.toFixed(0)} fetches=${fetches} mounted=${all[all.length - 1]?.mounted ?? "?"} scrollTop=${all[all.length - 1]?.scrollTop.toFixed(0) ?? "?"}`,
    );
    if (batch.end === "top") {
      // We are at scrollTop=0. A fetchOlder page may be in flight (the 600px
      // sentinel fires well before the top). Wait for the prepend to commit,
      // measure the FELT jump it causes on a visible row, then keep walking
      // into the paged-in history. Only a timeout means true top-of-history.
      const beforeWait = await timeline.evaluate((element) => {
        const el = element as HTMLDivElement;
        const first = el.querySelector<HTMLElement>("[data-message-id]");
        return {
          mounted: el.querySelectorAll("[data-message-id]").length,
          scrollTop: el.scrollTop,
          firstId: first?.dataset.messageId ?? null,
          firstTop: first?.getBoundingClientRect().top ?? 0,
        };
      });
      const grew = await page
        .waitForFunction(
          (n) =>
            document.querySelectorAll(
              '[data-testid="message-timeline"] [data-message-id]',
            ).length > n,
          beforeWait.mounted,
          { timeout: 15_000 },
        )
        .then(() => true)
        .catch(() => false);
      if (!grew) {
        ended = "top-of-history";
        break;
      }
      await page.waitForTimeout(300); // let the re-anchor settle
      const afterWait = await timeline.evaluate(
        (element, firstId: string | null) => {
          const el = element as HTMLDivElement;
          const row = firstId
            ? el.querySelector<HTMLElement>(
                `[data-message-id="${CSS.escape(firstId)}"]`,
              )
            : null;
          return {
            mounted: el.querySelectorAll("[data-message-id]").length,
            scrollTop: el.scrollTop,
            firstTop: row?.getBoundingClientRect().top ?? null,
          };
        },
        beforeWait.firstId,
      );
      const felt =
        afterWait.firstTop !== null
          ? afterWait.firstTop - beforeWait.firstTop
          : null;
      console.log(
        `[prepend-commit] mounted ${beforeWait.mounted}->${afterWait.mounted} scrollTop ${beforeWait.scrollTop.toFixed(0)}->${afterWait.scrollTop.toFixed(0)} FELT-JUMP of anchored row=${felt === null ? "row-gone" : `${felt.toFixed(1)}px`}`,
      );
      prependCommits.push({
        step: stepIndex,
        mountedFrom: beforeWait.mounted,
        mountedTo: afterWait.mounted,
        scrollTopTo: afterWait.scrollTop,
        feltJump: felt,
      });
      continue;
    }
    if (fetches >= MIN_FETCHES && walked >= MIN_PX) {
      ended = "target-reached";
      break;
    }
  }

  // ---- REPORT ----
  const scored = all.filter((s) => s.move !== null) as Array<
    Step & { move: number }
  >;
  const hist = new Map<number, number>();
  for (const s of scored) {
    const b = Math.round(s.move);
    hist.set(b, (hist.get(b) ?? 0) + 1);
  }
  console.log(
    `\n=== LIVE UPSCROLL PROFILE (step=${STEP_PX}px): #${TARGET_CHANNEL} ===`,
  );
  console.log(
    `relay=${RELAY_HTTP} steps=${all.length} scored=${scored.length} ended=${ended}`,
  );
  console.log(
    `total input px=${all.length} · applied px=${all.reduce((s, x) => s + Math.max(0, x.appliedTop), 0)} · fetchOlder pages=${all.length ? all[all.length - 1].fetch : 0}`,
  );
  console.log("\n--- movement histogram (px moved per 1px input, rounded) ---");
  for (const [k, v] of [...hist.entries()].sort((a, b) => a[0] - b[0]))
    console.log(`  ${String(k).padStart(6)}px : ${v}`);

  const smooth = scored.filter(
    (s) => Math.abs(s.move - STEP_PX) <= Math.max(0.5, STEP_PX * 0.1),
  ).length;
  console.log(
    `\nsmooth steps (move≈STEP=${STEP_PX}px): ${smooth}/${scored.length} = ${((100 * smooth) / Math.max(1, scored.length)).toFixed(2)}%`,
  );

  const jumps = scored
    .filter((s) => Math.abs(s.move - STEP_PX) > Math.max(2, STEP_PX * 0.25))
    .sort((a, b) => Math.abs(b.move) - Math.abs(a.move));
  console.log(
    `\n--- jumps |move-STEP| > max(2, STEP*0.25): ${jumps.length} ---`,
  );
  let prevFetch = 0;
  const fetchSteps: number[] = [];
  for (const s of all) {
    if (s.fetch > prevFetch) fetchSteps.push(s.i);
    prevFetch = s.fetch;
  }
  console.log(`fetchOlder landed at steps: ${fetchSteps.join(", ")}`);
  for (const s of jumps.slice(0, 40))
    console.log(
      `  step=${s.i} move=${s.move.toFixed(1)}px applied=${s.appliedTop.toFixed(1)} mounted=${s.mounted} fetch=${s.fetch} scrollTop=${s.scrollTop.toFixed(0)} repick=${s.repick}`,
    );

  console.log(
    `\n--- prepend commits observed at top (${prependCommits.length}) ---`,
  );
  for (const c of prependCommits)
    console.log(
      `  step=${c.step} mounted ${c.mountedFrom}->${c.mountedTo} scrollTopAfter=${c.scrollTopTo.toFixed(0)} feltJump=${c.feltJump === null ? "row-gone" : `${c.feltJump.toFixed(1)}px`}`,
    );

  // Prepend re-anchors (scrollTop jumped up) for the record.
  const reanchors = all.filter((s) => s.appliedTop < -50);
  console.log(
    `\n--- prepend re-anchors (appliedTop < -50): ${reanchors.length} ---`,
  );
  for (const s of reanchors.slice(0, 20))
    console.log(
      `  step=${s.i} appliedTop=${s.appliedTop.toFixed(0)} mounted=${s.mounted} fetch=${s.fetch} scrollTop=${s.scrollTop.toFixed(0)}`,
    );
  console.log("==============================================\n");

  expect(all.length * STEP_PX).toBeGreaterThan(1000);
});
