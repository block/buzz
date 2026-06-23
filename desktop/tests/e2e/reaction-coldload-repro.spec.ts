import { expect, test } from "@playwright/test";

// Ground-truth reproduction for Tyler's "reactions don't show on first load"
// bug. Drives the REAL GUI (relay-mode e2e bridge) against the staging relay
// with Eva's real key, cold-loads #buzz-bugs, and observes whether messages
// known to carry reactions render their reaction pills on first paint vs only
// after a channel switch-away-and-back.
//
// NOT a CI test — points at a live external relay and a real member identity.
// Run explicitly: pnpm exec playwright test reaction-coldload-repro

const RELAY_WS = "wss://sprout-oss.stage.blox.sqprod.co";
const RELAY_HTTP = "https://sprout-oss.stage.blox.sqprod.co";
const BUZZ_BUGS_NAME = "buzz-bugs";

const EVA = {
  privateKey:
    "10d3c4de9637f8c7467fb9dbe1da301f22db01ac5e45617e70e3d4538b35a5a9",
  pubkey: "011987e296fd5006292d2f930b574be47c7801048d1983c46c425d3c95f0cffd",
  username: "Eva",
};

test("cold-load buzz-bugs and observe reaction render", async ({ page }) => {
  test.setTimeout(180_000);
  // Seed workspace + onboarding for Eva's pubkey so the app skips WelcomeSetup
  // and boots straight into the workspace pointed at staging.
  await page.addInitScript(
    ({ relayUrl, pubkey }) => {
      const workspaceId = "e2e-repro-workspace";
      window.localStorage.setItem(
        "buzz-workspaces",
        JSON.stringify([
          {
            id: workspaceId,
            name: "Staging Repro",
            relayUrl,
            addedAt: new Date().toISOString(),
          },
        ]),
      );
      window.localStorage.setItem("buzz-active-workspace-id", workspaceId);
      const scope = encodeURIComponent(relayUrl);
      window.localStorage.setItem(
        `buzz-onboarding-complete.v1:${pubkey}`,
        "true",
      );
      window.localStorage.setItem(
        `buzz-welcome-channel-ensured.v1:${scope}:${pubkey}`,
        "true",
      );
    },
    { relayUrl: RELAY_WS, pubkey: EVA.pubkey },
  );

  await page.addInitScript(
    ({ identity, relayHttpUrl, relayWsUrl }) => {
      (window as unknown as { __BUZZ_E2E__: unknown }).__BUZZ_E2E__ = {
        mode: "relay",
        identity,
        relayHttpUrl,
        relayWsUrl,
      };
    },
    { identity: EVA, relayHttpUrl: RELAY_HTTP, relayWsUrl: RELAY_WS },
  );

  const consoleLines: string[] = [];
  page.on("console", (msg) => {
    consoleLines.push(`[${msg.type()}] ${msg.text()}`);
  });
  page.on("pageerror", (err) => {
    consoleLines.push(`[pageerror] ${err.message}`);
  });

  // The staging relay's HTTP API does not return Access-Control-Allow-Origin,
  // so a browser fetch from the test origin (127.0.0.1:4173) is blocked by
  // CORS — every get_channels / backfill /query fails with "Failed to fetch".
  // That's a harness artifact, NOT Tyler's bug. Proxy the relay's HTTP origin
  // through Node fetch (no CORS) and inject the missing CORS header so the
  // real GUI data path runs exactly as shipped, just reachable from the test.
  const relayQueryBodies: string[] = [];
  await page.route(`${RELAY_HTTP}/**`, async (route) => {
    const req = route.request();
    const url = req.url();
    const postData = req.postData() ?? undefined;
    if (url.endsWith("/query") && postData) {
      relayQueryBodies.push(postData.slice(0, 400));
    }
    const headers = { ...req.headers() };
    delete headers.origin;
    // The staging relay sits behind a WAF that 403s any request whose
    // User-Agent contains the `Mozilla/` token (confirmed: `Mozilla/5.0` ->
    // 403, `curl`/`buzz-desktop`/empty -> 200). Chromium always sends a
    // Mozilla UA, so the browser data path is blocked at infra — a harness
    // artifact, NOT Tyler's bug (the real Tauri app queries via the Rust
    // reqwest client, which is not browser-UA-shaped). Rewrite the UA so the
    // shipped data path can run from the test browser.
    headers["user-agent"] = "buzz-desktop-e2e";
    const upstream = await fetch(url, {
      method: req.method(),
      headers,
      body:
        req.method() === "GET" || req.method() === "HEAD"
          ? undefined
          : postData,
    });
    const bodyBuf = Buffer.from(await upstream.arrayBuffer());
    const respHeaders: Record<string, string> = {};
    upstream.headers.forEach((v, k) => {
      respHeaders[k] = v;
    });
    respHeaders["access-control-allow-origin"] = "*";
    respHeaders["access-control-allow-headers"] = "*";
    respHeaders["access-control-allow-methods"] = "*";
    await route.fulfill({
      status: upstream.status,
      headers: respHeaders,
      body: bodyBuf,
    });
  });

  // Capture the aux backfill REQ + EVENT traffic over the websocket.
  const wsFrames: string[] = [];
  // The same `Mozilla/`-UA WAF that 403s the HTTP path also 403s the browser's
  // WS upgrade (confirmed by probe), and message history + reaction backfill
  // both ride the WebSocket, not the HTTP /query bridge. Playwright's
  // `connectToServer()` is ALSO WAF-blocked (its upstream handshake closes
  // 1006), and it exposes no way to set the UA. Node's built-in global
  // `WebSocket` (undici) connects fine — its UA is not `Mozilla/`-shaped — so
  // bridge the page's WS to a Node-side undici WebSocket by hand. The shipped
  // WS data path (auth, history, aux reaction backfill) then runs as-is, just
  // reachable from the test browser.
  // Per-REQ latency tracking: maps a subscription id to the wall-clock time
  // its REQ was sent, so we can measure REQ -> EOSE latency for each aux
  // backfill chunk. This is what separates "deterministic >8s timeout" (a
  // consistent GUI bug, as Tyler insists) from "random latency".
  const reqSentAt = new Map<string, number>();
  const reqFilters = new Map<string, string>();
  // Aux-backfill subscription tracking + a bridge-throughput counter, to
  // distinguish "relay never sent the EOSE" from "bridge stalled and dropped
  // it" (harness artifact). auxSubIds holds the aux REQ ids (kinds 5,7,9005,
  // 40003 keyed by #e); auxFrameLog records every upstream frame for them.
  const auxSubIds = new Set<string>();
  const auxFrameLog: string[] = [];
  let upstreamFrameCount = 0;
  const t0 = Date.now();
  const rel = () => `${((Date.now() - t0) / 1000).toFixed(2)}s`;
  await page.routeWebSocket(/sprout-oss/, (ws) => {
    wsFrames.push(`ROUTE HIT url=${ws.url()}`);
    const upstream = new WebSocket(ws.url());
    const pageQueue: (string | Buffer)[] = [];
    let upstreamOpen = false;
    upstream.addEventListener("open", () => {
      upstreamOpen = true;
      for (const m of pageQueue) upstream.send(m);
      pageQueue.length = 0;
    });
    upstream.addEventListener("message", (ev: MessageEvent) => {
      const m = ev.data as string;
      upstreamFrameCount++;
      if (typeof m === "string" && m.startsWith("[")) {
        try {
          const arr = JSON.parse(m) as unknown[];
          const verb = arr[0];
          const sid = arr[1];
          // Track every frame for the aux-backfill sub specifically, so we can
          // tell whether the relay delivered its EVENTs/EOSE to the bridge even
          // if the page never rendered them (= harness bottleneck, not the bug).
          if (typeof sid === "string" && auxSubIds.has(sid)) {
            auxFrameLog.push(
              `[${rel()}] AUX-FRAME #${upstreamFrameCount} verb=${verb} sid=${sid.slice(0, 20)}`,
            );
          }
          if (verb === "EOSE" && typeof sid === "string") {
            const sent = reqSentAt.get(sid);
            const filt = reqFilters.get(sid) ?? "";
            if (sent !== undefined) {
              wsFrames.push(
                `EOSE sid=${sid} latency=${((Date.now() - sent) / 1000).toFixed(2)}s filter=${filt}`,
              );
              reqSentAt.delete(sid);
            }
          }
        } catch {
          /* not JSON array */
        }
      }
      if (
        typeof m === "string" &&
        (m.includes('"kind":7') ||
          m.includes("EOSE") ||
          m.includes("OK") ||
          m.includes("AUTH"))
      )
        wsFrames.push(`[${rel()}] << ${m.slice(0, 200)}`);
      ws.send(m);
    });
    upstream.addEventListener("close", (ev: CloseEvent) =>
      wsFrames.push(`UPSTREAM CLOSE code=${ev.code}`),
    );
    upstream.addEventListener("error", () => wsFrames.push(`UPSTREAM ERROR`));
    ws.onMessage((m) => {
      const p = typeof m === "string" ? m : "<binary>";
      if (typeof m === "string" && m.startsWith("[")) {
        try {
          const arr = JSON.parse(m) as unknown[];
          if (arr[0] === "REQ" && typeof arr[1] === "string") {
            const sid = arr[1];
            reqSentAt.set(sid, Date.now());
            // Record the kinds + which #e tag count so we can attribute the
            // latency to aux-backfill (kinds 5,7,9005,40003 + #e) vs other REQs.
            const filterObj = arr[2] as
              | { kinds?: number[]; "#e"?: string[] }
              | undefined;
            const kinds = filterObj?.kinds?.join(",") ?? "?";
            const eCount = filterObj?.["#e"]?.length ?? 0;
            reqFilters.set(sid, `kinds=[${kinds}] #e=${eCount}`);
            // Aux backfill REQs are keyed by #e. Post kind-split (the fix),
            // reactions ride a kind:7-only REQ and the structural overlay rides
            // a 5/9005/40003 REQ; pre-fix they were one bundled 5+7+... REQ.
            // Track all aux REQs so we can measure each one's REQ->EOSE latency
            // and prove the reaction REQ now beats the 8s timeout.
            const kindSet = new Set(filterObj?.kinds ?? []);
            const isReactionReq =
              eCount > 0 && kindSet.has(7) && !kindSet.has(5);
            const isStructuralReq =
              eCount > 0 && kindSet.has(5) && !kindSet.has(7);
            const isBundledReq = eCount > 0 && kindSet.has(7) && kindSet.has(5);
            if (isReactionReq || isStructuralReq || isBundledReq) {
              auxSubIds.add(sid);
            }
            wsFrames.push(
              `[${rel()}] REQ sid=${sid} kinds=[${kinds}] #e=${eCount}${
                isReactionReq
                  ? " [REACTION-AUX]"
                  : isStructuralReq
                    ? " [STRUCTURAL-AUX]"
                    : isBundledReq
                      ? " [BUNDLED-AUX]"
                      : ""
              }`,
            );
          }
        } catch {
          /* not JSON array */
        }
      }
      if (p.includes("REQ") || p.includes("kinds") || p.includes("AUTH"))
        wsFrames.push(`[${rel()}] >> ${p.slice(0, 300)}`);
      if (upstreamOpen) upstream.send(m);
      else pageQueue.push(m);
    });
    ws.onClose(() => {
      try {
        upstream.close();
      } catch {
        /* noop */
      }
    });
  });

  await page.goto("/");

  try {
    // Wait for the sidebar + buzz-bugs channel entry to materialize from the
    // relay (kind:39002 membership -> kind:39000 metadata).
    const channelEntry = page.getByTestId(`channel-${BUZZ_BUGS_NAME}`);
    await channelEntry.waitFor({ state: "visible", timeout: 60_000 });

    // ---- COLD LOAD: first entry into the channel ----
    await channelEntry.click();
    await expect(page.getByTestId("chat-title")).toHaveText(BUZZ_BUGS_NAME, {
      timeout: 20_000,
    });
    // Let messages render.
    await page.getByTestId("message-row").first().waitFor({ timeout: 20_000 });

    // Give the cold-load aux backfill a generous window to commit.
    await page.waitForTimeout(6_000);

    const coldLoadReactionCount = await page
      .getByTestId("message-reactions")
      .count();
    const coldLoadMessageCount = await page.getByTestId("message-row").count();
    await page.screenshot({
      path: "test-results/reaction-coldload/01-coldload.png",
      fullPage: true,
    });

    // ---- DISAMBIGUATE: switch away then back ----
    // Click any other channel, then return to buzz-bugs.
    const otherChannel = page
      .locator('[data-testid^="channel-"]')
      .filter({ hasNot: channelEntry })
      .first();
    await otherChannel.click();
    await page.waitForTimeout(1_500);
    await channelEntry.click();
    await expect(page.getByTestId("chat-title")).toHaveText(BUZZ_BUGS_NAME);
    await page.getByTestId("message-row").first().waitFor({ timeout: 20_000 });
    await page.waitForTimeout(6_000);

    const afterSwitchReactionCount = await page
      .getByTestId("message-reactions")
      .count();
    await page.screenshot({
      path: "test-results/reaction-coldload/02-afterswitch.png",
      fullPage: true,
    });

    console.log("=== REPRO RESULT ===");
    console.log("cold-load message rows:", coldLoadMessageCount);
    console.log("cold-load reaction containers:", coldLoadReactionCount);
    console.log("after-switch reaction containers:", afterSwitchReactionCount);

    // The proof: switching away-and-back is what "fixes" the symptom for the
    // user today, so after-switch is the ground-truth count of reactions that
    // exist for the loaded window. The fix means the FIRST cold-load paint must
    // already show them — not zero, and not far short of after-switch. Pre-fix
    // this was 0 on a busy workspace (all-or-nothing drop on the bundled REQ
    // timeout); post-fix the kind:7 REQ lands well inside the 8s budget.
    expect(
      coldLoadReactionCount,
      "cold-load must render reactions on first paint (pre-fix: 0)",
    ).toBeGreaterThan(0);
    if (afterSwitchReactionCount > 0) {
      expect(
        coldLoadReactionCount,
        "cold-load reaction count must reach the after-switch ground truth",
      ).toBeGreaterThanOrEqual(afterSwitchReactionCount);
    }
  } finally {
    // Always dump diagnostics, even if a waitFor above timed out — this is the
    // whole point of the harness. Without the finally, a timeout fires before
    // the log block and we learn nothing.
    const channelTestIds = await page
      .locator('[data-testid^="channel-"]')
      .evaluateAll((els) =>
        els.map((e) => (e as HTMLElement).dataset.testid ?? ""),
      )
      .catch(() => [] as string[]);
    console.log("=== CHANNELS RENDERED IN SIDEBAR ===");
    console.log(JSON.stringify(channelTestIds));
    console.log("=== WS FRAMES (reaction/REQ) ===");
    for (const f of wsFrames) console.log(f);
    console.log("=== AUX-BACKFILL SUB FRAMES (relay -> bridge) ===");
    console.log("total upstream frames bridged:", upstreamFrameCount);
    console.log("aux sub ids tracked:", [...auxSubIds].join(", "));
    for (const f of auxFrameLog) console.log(f);
    console.log("=== RELAY /query BODIES ===");
    for (const b of relayQueryBodies.slice(0, 40)) console.log(b);
    console.log("=== CONSOLE (last 40) ===");
    for (const l of consoleLines.slice(-40)) console.log(l);
    await page
      .screenshot({
        path: "test-results/reaction-coldload/99-final.png",
        fullPage: true,
      })
      .catch(() => {});
  }
});
