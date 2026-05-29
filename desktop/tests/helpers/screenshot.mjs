#!/usr/bin/env node
//
// Standalone Playwright screenshot helper for the Sprout desktop app.
//
// Launches headless Chromium with the E2E mock bridge pre-injected (same
// setup as installMockBridge in bridge.ts), navigates to a route, optionally
// clicks an element, and saves a screenshot.
//
// Usage:
//   node tests/helpers/screenshot.mjs [options]
//
// Options:
//   --name <name>        Screenshot filename without extension (default: screenshot)
//   --route <path>       Client-side route to navigate to (default: /)
//   --click <selector>   CSS selector or data-testid to click before capture
//   --wait <ms>          Milliseconds to wait before capture (default: 2000)
//   --viewport <WxH>     Viewport dimensions (default: 1280x720)
//   --outdir <path>      Output directory (default: test-results/screenshots)

import { parseArgs } from "node:util";
import { existsSync, mkdirSync } from "node:fs";
import { resolve, join } from "node:path";
import { chromium } from "@playwright/test";

const { values: args } = parseArgs({
  options: {
    name: { type: "string", default: "screenshot" },
    route: { type: "string", default: "/" },
    click: { type: "string" },
    wait: { type: "string", default: "2000" },
    viewport: { type: "string", default: "1280x720" },
    outdir: { type: "string", default: "test-results/screenshots" },
  },
  strict: true,
});

const [vpWidth, vpHeight] = args.viewport.split("x").map(Number);
const waitMs = Number(args.wait);
const outdir = resolve(args.outdir);

if (!existsSync(outdir)) {
  mkdirSync(outdir, { recursive: true });
}

const BASE_URL = "http://127.0.0.1:4173";
const DEFAULT_MOCK_PUBKEY = "deadbeef".repeat(8);
const ONBOARDING_PREFIX = "sprout-onboarding-complete.v1:";

const TEST_PUBKEYS = [
  DEFAULT_MOCK_PUBKEY,
  "e5ebc6cdb579be112e336cc319b5989b4bb6af11786ea90dbe52b5f08d741b34",
  "953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f",
  "bb22a5299220cad76ffd46190ccbeede8ab5dc260faa28b6e5a2cb31b9aff260",
  "554cef57437abac34522ac2c9f0490d685b72c80478cf9f7ed6f9570ee8624ea",
  "df8e91b86fda13a9a67896df77232f7bdab2ba9c3e165378e1ba3d24c13a328e",
];

const browser = await chromium.launch({ headless: true });
const page = await browser.newPage({
  viewport: { width: vpWidth, height: vpHeight },
});

// Seed default workspace (mirrors seedDefaultWorkspace in bridge.ts)
await page.addInitScript(() => {
  const workspaceId = "e2e-default-workspace";
  const workspace = {
    id: workspaceId,
    name: "E2E Test",
    relayUrl: "ws://localhost:3000",
    addedAt: new Date().toISOString(),
  };
  window.localStorage.setItem("sprout-workspaces", JSON.stringify([workspace]));
  window.localStorage.setItem("sprout-active-workspace-id", workspaceId);
});

// Seed onboarding completion for all known identities
await page.addInitScript(
  ({ prefix, pubkeys }) => {
    for (const pk of pubkeys) {
      window.localStorage.setItem(`${prefix}${pk}`, "true");
    }
  },
  { prefix: ONBOARDING_PREFIX, pubkeys: TEST_PUBKEYS },
);

// Install E2E mock bridge config + MockNotification (mirrors installBridge in bridge.ts)
await page.addInitScript(() => {
  class MockNotification extends EventTarget {
    static permission = "granted";
    static async requestPermission() {
      return "granted";
    }
    body;
    onclick = null;
    title;
    constructor(title, options) {
      super();
      this.title = title;
      this.body = options?.body ?? null;
    }
    close() {}
  }
  Object.defineProperty(window, "Notification", {
    configurable: true,
    value: MockNotification,
    writable: true,
  });

  window.__SPROUT_E2E__ = { mode: "mock" };
  window.__SPROUT_E2E_APP_BADGE_COUNT__ = 0;
});

const url = args.route === "/" ? BASE_URL : `${BASE_URL}/#${args.route}`;
await page.goto(url);
await page.waitForTimeout(waitMs);

if (args.click) {
  const selector = args.click.startsWith("[")
    ? args.click
    : `[data-testid="${args.click}"]`;
  await page.click(selector);
  await page.waitForTimeout(500);
}

const filepath = join(outdir, `${args.name}.png`);
await page.screenshot({ path: filepath });
console.log(filepath);

await browser.close();
