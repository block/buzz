// Custom screenshot helper for header zoom-fix comparison.
// Supports --port (4173 baseline / 4174 branch), --zoom (CSS zoom factor), and --open-thread / --open-profile / --open-agent.
import { chromium } from "@playwright/test";
import { mkdirSync } from "node:fs";
import { join, resolve } from "node:path";
import { parseArgs } from "node:util";

const { values: a } = parseArgs({
  options: {
    name: { type: "string", default: "shot" },
    port: { type: "string", default: "4173" },
    channel: { type: "string", default: "general" },
    zoom: { type: "string", default: "1" },
    "open-thread": { type: "boolean", default: false },
    "open-profile": { type: "boolean", default: false },
    "open-agent": { type: "boolean", default: false },
    viewport: { type: "string", default: "1280x800" },
    clip: { type: "string" },
    wait: { type: "string", default: "1500" },
    outdir: { type: "string", default: "/tmp/zoom-shots" },
  },
  strict: true,
});

const [vw, vh] = a.viewport.split("x").map(Number);
mkdirSync(resolve(a.outdir), { recursive: true });

const BASE = `http://127.0.0.1:${a.port}`;
const DEFAULT_MOCK_PUBKEY = "deadbeef".repeat(8);
const ONBOARDING_PREFIX = "buzz-onboarding-complete.v1:";
const TEST_PUBKEYS = [
  DEFAULT_MOCK_PUBKEY,
  "e5ebc6cdb579be112e336cc319b5989b4bb6af11786ea90dbe52b5f08d741b34",
  "953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f",
  "bb22a5299220cad76ffd46190ccbeede8ab5dc260faa28b6e5a2cb31b9aff260",
  "554cef57437abac34522ac2c9f0490d685b72c80478cf9f7ed6f9570ee8624ea",
  "df8e91b86fda13a9a67896df77232f7bdab2ba9c3e165378e1ba3d24c13a328e",
];

const browser = await chromium.launch({ headless: true });
const page = await browser.newPage({ viewport: { width: vw, height: vh } });

await page.addInitScript(() => {
  const id = "e2e-default-community";
  const ws = {
    id,
    name: "E2E Test",
    relayUrl: "ws://localhost:3000",
    addedAt: new Date().toISOString(),
  };
  window.localStorage.setItem("buzz-communities", JSON.stringify([ws]));
  window.localStorage.setItem("buzz-active-community-id", id);
});
await page.addInitScript(
  ({ prefix, pubkeys }) => {
    for (const pk of pubkeys)
      window.localStorage.setItem(`${prefix}${pk}`, "true");
  },
  { prefix: ONBOARDING_PREFIX, pubkeys: TEST_PUBKEYS },
);
await page.addInitScript(() => {
  class MockNotification extends EventTarget {
    static permission = "granted";
    static async requestPermission() {
      return "granted";
    }
    constructor(t, o) {
      super();
      this.title = t;
      this.body = o?.body ?? null;
      this.onclick = null;
    }
    close() {}
  }
  Object.defineProperty(window, "Notification", {
    configurable: true,
    value: MockNotification,
    writable: true,
  });
  window.__BUZZ_E2E__ = { mode: "mock" };
  window.__BUZZ_E2E_APP_BADGE_COUNT__ = 0;
});

try {
  await page.goto(BASE);
  await page.waitForSelector(`[data-testid="channel-${a.channel}"]`, {
    timeout: 15000,
  });
  await page.click(`[data-testid="channel-${a.channel}"]`);
  await page.waitForSelector(`[data-testid="chat-title"]`, { timeout: 10000 });
  await page.waitForTimeout(800);

  if (a["open-profile"]) {
    // Click chat-title or members trigger to open profile? Better: avatar in chat header.
    const avatar = page
      .locator('[data-testid="chat-header"] [data-testid^="avatar-"]')
      .first();
    if ((await avatar.count()) > 0) await avatar.click();
  }

  if (a["open-thread"]) {
    // Send a message via mock so we have something to thread.
    await page.evaluate(
      ({ ch }) => {
        window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
          channelName: ch,
          content: "Header spacing comparison anchor message",
          pubkey: "deadbeef".repeat(8),
        });
      },
      { ch: a.channel },
    );
    await page.waitForTimeout(400);
    // Hover the message and click its reply trigger.
    const replyBtn = page.locator('[data-testid^="reply-message-"]').first();
    if ((await replyBtn.count()) > 0) {
      await replyBtn.scrollIntoViewIfNeeded();
      await replyBtn.click({ force: true });
      await page.waitForTimeout(600);
    }
  }

  // Apply CSS zoom on the html element AFTER interaction setup.
  const zoom = Number(a.zoom);
  if (zoom !== 1) {
    await page.evaluate((z) => {
      document.documentElement.style.zoom = String(z);
    }, zoom);
    await page.waitForTimeout(300);
  }

  await page.waitForTimeout(Number(a.wait));

  const filepath = join(resolve(a.outdir), `${a.name}.png`);
  const opts = {};
  if (a.clip) {
    const [x, y, w, h] = a.clip.split(",").map(Number);
    opts.clip = { x, y, width: w, height: h };
  }
  await page.screenshot({ path: filepath, ...opts });
  console.log(filepath);
} finally {
  await browser.close();
}
