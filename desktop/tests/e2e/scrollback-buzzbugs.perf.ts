import { expect, test } from "@playwright/test";
import { decode } from "nostr-tools/nip19";
import { getPublicKey } from "nostr-tools/pure";

import { installRelayBridge } from "../helpers/bridge";

/**
 * SCROLL-BACK latency profile for one channel (#buzz-bugs) against a LIVE
 * relay, post-PR #1500 read-model.
 *
 * Mechanics under test (source: useLoadOlderOnScroll.ts, pageOlderMessages.ts,
 * e2eBridge.ts handleGetChannelWindow):
 *   - IntersectionObserver on a top sentinel, rootMargin 600px, armed once
 *     per leave->enter gesture.
 *   - fetchOlder -> pageOlderMessagesUntilRowFloor: EXACTLY ONE 50-row page
 *     per trigger, in-flight dedupe per channel, composite (until, before_id)
 *     keyset cursor.
 *   - Page response = rows + aux + thread summaries + one kind-39006 bounds.
 *
 * We measure, per page: gesture->fetch-trigger ms, network RTT (Node-side,
 * excludes browser/CORS shim), payload bytes, kind histogram (rows vs aux vs
 * summaries bloat), fetch-resolve->rows-committed ms, and scroll anchoring.
 * Plus: every relay HTTP call during the scroll phase (duplicate detection),
 * console errors, longtasks.
 *
 * Run (in-cluster port-forward, Host rewritten to community host):
 *   kubectl -n sprout port-forward svc/sprout-relay 13000:3000 &
 *   BUZZ_E2E_RELAY_URL=http://127.0.0.1:13000 \
 *   BUZZ_COMMUNITY_HOST=sprout-oss.stage.blox.sqprod.co \
 *   BUZZ_PERF_NSEC=nsec1... \
 *   npx playwright test --config=playwright.perf.config.ts scrollback-buzzbugs.perf.ts
 */

const RELAY_HTTP =
  process.env.BUZZ_E2E_RELAY_URL ?? "https://sprout-oss.stage.blox.sqprod.co";
const NSEC = process.env.BUZZ_PERF_NSEC ?? "";
const COMMUNITY_HOST = process.env.BUZZ_COMMUNITY_HOST ?? "";
const TARGET_CHANNEL = process.env.BUZZ_PERF_CHANNEL ?? "buzz-bugs";
const PAGES = Number(process.env.BUZZ_PERF_PAGES ?? 10);

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
  const privateKey = Buffer.from(skBytes).toString("hex");
  const pubkey = getPublicKey(skBytes);
  return { privateKey, pubkey, username: "perf-eva" };
}

type HttpSample = {
  seq: number;
  path: string;
  tStart: number;
  ms: number;
  bytes: number;
  status: number;
  body: string;
  kinds: Record<string, number>;
  eventCount: number;
};

function kindHistogram(buf: Buffer): {
  kinds: Record<string, number>;
  eventCount: number;
} {
  const kinds: Record<string, number> = {};
  let eventCount = 0;
  try {
    const parsed = JSON.parse(buf.toString("utf8"));
    const arr = Array.isArray(parsed) ? parsed : parsed?.events;
    if (Array.isArray(arr)) {
      for (const e of arr) {
        if (e && typeof e.kind === "number") {
          kinds[String(e.kind)] = (kinds[String(e.kind)] ?? 0) + 1;
          eventCount++;
        }
      }
    }
  } catch {
    /* non-JSON */
  }
  return { kinds, eventCount };
}

test("MEASURE: scroll-back pagination latency in target channel", async ({
  page,
}) => {
  test.setTimeout(300_000);
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

  // longtask observer
  await page.addInitScript(() => {
    const store = window as unknown as { __LONGTASKS__?: number[] };
    store.__LONGTASKS__ = [];
    new PerformanceObserver((list) => {
      for (const e of list.getEntries()) store.__LONGTASKS__?.push(e.duration);
    }).observe({ type: "longtask", buffered: true });
  });

  // Relay HTTP interception: doorman bypass (UA + strip sec-ch-ua), Host
  // rewrite for in-cluster port-forward, ACAO injection, Node-side RTT +
  // payload kind histogram capture. Same approach as staging-latency.perf.ts.
  const relayHost = new URL(RELAY_HTTP).host;
  const samples: HttpSample[] = [];
  const runStart = Date.now();
  await page.route(
    (url) => url.host === relayHost,
    async (route) => {
      const req = route.request();
      const t0 = Date.now();
      const fwd = { ...req.headers(), "user-agent": REAL_CHROME_UA };
      delete fwd["sec-ch-ua"];
      delete fwd["sec-ch-ua-mobile"];
      delete fwd["sec-ch-ua-platform"];
      if (COMMUNITY_HOST) fwd.host = COMMUNITY_HOST;
      const resp = await route.fetch({ headers: fwd });
      const ms = Date.now() - t0;
      const buf = await resp.body();
      const { kinds, eventCount } = kindHistogram(buf);
      samples.push({
        seq: samples.length,
        path: new URL(req.url()).pathname,
        tStart: t0 - runStart,
        ms,
        bytes: buf.byteLength,
        status: resp.status(),
        body: req.postData() ?? "",
        kinds,
        eventCount,
      });
      const headers = { ...resp.headers() };
      headers["access-control-allow-origin"] = "*";
      headers["access-control-allow-headers"] = "*";
      headers["access-control-allow-methods"] = "*";
      await route.fulfill({ response: resp, headers, body: buf });
    },
  );

  const consoleLines: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning")
      consoleLines.push(`[${msg.type()}] ${msg.text()}`.slice(0, 300));
  });
  page.on("pageerror", (err) =>
    consoleLines.push(`[pageerror] ${err.message}`.slice(0, 300)),
  );

  // ---- Boot to sidebar, open the target channel ----
  await page.goto("/");
  await page.getByTestId("app-sidebar").waitFor({ state: "visible" });
  const chan = page.getByTestId(`channel-${TARGET_CHANNEL}`).first();
  await chan.waitFor({ state: "visible", timeout: 45_000 });

  const openMark = samples.length;
  const tOpen = Date.now();
  await chan.click();
  await page
    .locator('[data-testid="message-timeline"] [data-message-id]')
    .first()
    .waitFor({ state: "visible", timeout: 30_000 });
  const openWallMs = Date.now() - tOpen;

  // Let the initial burst (window + profiles + summaries) fully settle so the
  // scroll phase attribution is clean.
  await page.waitForTimeout(3000);
  const openSamples = samples.slice(openMark);

  type PageResult = {
    page: number;
    fired: boolean;
    steps: number;
    triggerMs: number; // gesture start -> bridge fetch counted
    networkMs: number; // Node-side /query RTT
    bytes: number;
    eventCount: number;
    kinds: Record<string, number>;
    commitMs: number; // fetch resolve -> new rows committed in DOM
    totalMs: number; // gesture start -> rows committed
    rowsBefore: number;
    rowsAfter: number;
    extraCalls: number; // relay HTTP calls beyond the page /query itself
    scrollTopAfter: number;
    longtaskMs: number;
  };
  const results: PageResult[] = [];

  for (let p = 1; p <= PAGES; p++) {
    const mark = samples.length;
    await page.evaluate(() => {
      (window as unknown as { __LONGTASKS__: number[] }).__LONGTASKS__ = [];
    });
    const rowsBefore = await page
      .locator('[data-testid="message-timeline"] [data-message-id]')
      .count();

    const t0 = Date.now();
    // Real scroll gesture: step scrollTop upward until the sentinel enters the
    // 600px IO band and the bridge probe counts a cursor fetch, or we hit top.
    const gesture = await page.evaluate(
      async ({ step, maxSteps }) => {
        const el = document.querySelector(
          '[data-testid="message-timeline"]',
        ) as HTMLDivElement | null;
        const probe = window as unknown as {
          __CHANNEL_WINDOW_FETCH_COUNT__?: number;
        };
        if (!el) return { fired: false, steps: 0, atTop: false };
        const c0 = probe.__CHANNEL_WINDOW_FETCH_COUNT__ ?? 0;
        for (let i = 0; i < maxSteps; i++) {
          el.scrollBy(0, -step);
          await new Promise((r) =>
            requestAnimationFrame(() => setTimeout(r, 25)),
          );
          if ((probe.__CHANNEL_WINDOW_FETCH_COUNT__ ?? 0) > c0)
            return { fired: true, steps: i + 1, atTop: el.scrollTop <= 0 };
          if (el.scrollTop <= 0) {
            await new Promise((r) => setTimeout(r, 300));
            return {
              fired: (probe.__CHANNEL_WINDOW_FETCH_COUNT__ ?? 0) > c0,
              steps: i + 1,
              atTop: true,
            };
          }
        }
        return { fired: false, steps: maxSteps, atTop: false };
      },
      { step: 300, maxSteps: 400 },
    );
    const triggerMs = Date.now() - t0;

    if (!gesture.fired) {
      console.log(
        `[page ${p}] no fetch fired after ${gesture.steps} steps (atTop=${gesture.atTop}) — history exhausted or observer stalled`,
      );
      results.push({
        page: p,
        fired: false,
        steps: gesture.steps,
        triggerMs,
        networkMs: 0,
        bytes: 0,
        eventCount: 0,
        kinds: {},
        commitMs: 0,
        totalMs: triggerMs,
        rowsBefore,
        rowsAfter: rowsBefore,
        extraCalls: samples.length - mark,
        scrollTopAfter: -1,
        longtaskMs: 0,
      });
      break;
    }

    // Wait for the prepended rows to commit.
    await page
      .waitForFunction(
        (n) =>
          document.querySelectorAll(
            '[data-testid="message-timeline"] [data-message-id]',
          ).length > n,
        rowsBefore,
        { timeout: 20_000 },
      )
      .catch(() => {});
    const totalMs = Date.now() - t0;
    await page.waitForTimeout(400); // let stragglers (profiles etc.) land

    const rowsAfter = await page
      .locator('[data-testid="message-timeline"] [data-message-id]')
      .count();
    const scrollTopAfter = await page.evaluate(
      () =>
        (
          document.querySelector(
            '[data-testid="message-timeline"]',
          ) as HTMLDivElement
        )?.scrollTop ?? -1,
    );
    const lts = await page.evaluate(
      () =>
        (window as unknown as { __LONGTASKS__: number[] }).__LONGTASKS__ ?? [],
    );

    const phase = samples.slice(mark);
    // The page fetch is the /query whose body carries the keyset cursor.
    const pageFetch = phase.find(
      (s) => s.path.includes("/query") && s.body.includes("before_id"),
    );
    results.push({
      page: p,
      fired: true,
      steps: gesture.steps,
      triggerMs,
      networkMs: pageFetch?.ms ?? 0,
      bytes: pageFetch?.bytes ?? 0,
      eventCount: pageFetch?.eventCount ?? 0,
      kinds: pageFetch?.kinds ?? {},
      commitMs: totalMs - triggerMs - (pageFetch?.ms ?? 0),
      totalMs,
      rowsBefore,
      rowsAfter,
      extraCalls: phase.length - (pageFetch ? 1 : 0),
      scrollTopAfter,
      longtaskMs: lts.reduce((s, d) => s + d, 0),
    });
    console.log(
      `[page ${p}] trigger=${triggerMs}ms net=${pageFetch?.ms ?? "?"}ms bytes=${pageFetch?.bytes ?? "?"} events=${pageFetch?.eventCount ?? "?"} commit=${(totalMs - triggerMs - (pageFetch?.ms ?? 0)).toFixed(0)}ms total=${totalMs}ms rows ${rowsBefore}->${rowsAfter} extraCalls=${phase.length - (pageFetch ? 1 : 0)}`,
    );
  }

  // ---- REPORT ----
  /* eslint-disable no-console */
  const fired = results.filter((r) => r.fired);
  const pctl = (vals: number[], p: number) => {
    if (!vals.length) return 0;
    const s = [...vals].sort((a, b) => a - b);
    return s[Math.min(s.length - 1, Math.floor((p / 100) * s.length))];
  };

  console.log(`\n=== SCROLL-BACK PROFILE: #${TARGET_CHANNEL} ===`);
  console.log(
    `relay: ${RELAY_HTTP}  identity: ${identity.pubkey.slice(0, 12)}…`,
  );
  console.log(
    `channel open: wall=${openWallMs}ms, ${openSamples.length} relay calls, ${openSamples.reduce((s, n) => s + n.bytes, 0)} bytes`,
  );
  console.log("\n--- initial channel-open call log ---");
  for (const s of openSamples)
    console.log(
      `  t+${s.tStart}ms ${s.status} ${s.path} ${s.ms}ms ${s.bytes}b events=${s.eventCount} kinds=${JSON.stringify(s.kinds)} body=${s.body.slice(0, 160)}`,
    );
  console.log("\n--- per-page scroll-back ---");
  console.log(
    `pages fired: ${fired.length}/${results.length}` +
      ` | net p50=${pctl(
        fired.map((r) => r.networkMs),
        50,
      )}ms p95=${pctl(
        fired.map((r) => r.networkMs),
        95,
      )}ms` +
      ` | total p50=${pctl(
        fired.map((r) => r.totalMs),
        50,
      )}ms p95=${pctl(
        fired.map((r) => r.totalMs),
        95,
      )}ms` +
      ` | bytes p50=${pctl(
        fired.map((r) => r.bytes),
        50,
      )}`,
  );
  for (const r of results) {
    console.log(
      `  page=${r.page} fired=${r.fired} steps=${r.steps} trigger=${r.triggerMs}ms net=${r.networkMs}ms commit=${r.commitMs.toFixed(0)}ms total=${r.totalMs}ms bytes=${r.bytes} events=${r.eventCount} rows=${r.rowsBefore}->${r.rowsAfter} extra=${r.extraCalls} scrollTop=${r.scrollTopAfter} longtask=${r.longtaskMs.toFixed(0)}ms kinds=${JSON.stringify(r.kinds)}`,
    );
  }

  // Full network log for the whole run (duplicate detection).
  console.log("\n--- FULL relay HTTP log ---");
  for (const s of samples)
    console.log(
      `  t+${s.tStart}ms ${s.status} ${s.path} ${s.ms}ms ${s.bytes}b events=${s.eventCount} body=${s.body.slice(0, 200)}`,
    );

  const exact = new Map<string, number>();
  for (const s of samples) {
    const key = `${s.path}|${s.body}`;
    exact.set(key, (exact.get(key) ?? 0) + 1);
  }
  console.log("\n--- DUPLICATE exact request bodies (count>1) ---");
  let dups = 0;
  for (const [k, n] of [...exact.entries()].sort((a, b) => b[1] - a[1])) {
    if (n > 1) {
      dups++;
      console.log(`  [${n}x] ${k.slice(0, 260)}`);
    }
  }
  if (!dups) console.log("  (none)");

  console.log(`\n--- console errors/warnings (${consoleLines.length}) ---`);
  for (const l of consoleLines.slice(0, 30)) console.log(`  ${l}`);
  console.log("==============================================\n");
  /* eslint-enable no-console */

  expect(fired.length).toBeGreaterThan(0);
});
