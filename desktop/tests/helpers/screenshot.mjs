#!/usr/bin/env node
//
// Standalone Playwright screenshot helper for the Buzz desktop app.
//
// Launches headless Chromium with the E2E mock bridge pre-injected (same
// setup as installMockBridge in bridge.ts), navigates to a route, optionally
// clicks an element, and saves a screenshot.
//
// Usage:
//   node tests/helpers/screenshot.mjs [options]
//
// Options:
//   --name <name>              Screenshot filename without extension (default: screenshot)
//   --route <path>             Client-side route to navigate to (default: /)
//   --active-channel <name>    Channel to navigate to and view (channel-aware navigation)
//   --click <selector>         CSS selector or data-testid to click before capture
//   --right-click <selector>   Right-click a selector (for context menus)
//   --hover <selector>         Hover over a selector before capture
//   --clip <x,y,w,h>           Crop to a region (e.g. 0,0,256,720 for sidebar)
//   --wait <ms>                Milliseconds to wait before capture (default: 2000)
//   --viewport <WxH>           Viewport dimensions (default: 1280x720)
//   --outdir <path>            Output directory (default: test-results/screenshots)
//   --messages <path>          JSON file with messages to inject before capture
//   --update-ready             Mock an available update so the sidebar update card renders

import { parseArgs } from "node:util";
import { existsSync, mkdirSync, readFileSync } from "node:fs";
import { resolve, join } from "node:path";
import { chromium } from "@playwright/test";

const { values: args } = parseArgs({
  options: {
    name: { type: "string", default: "screenshot" },
    route: { type: "string", default: "/" },
    "active-channel": { type: "string" },
    click: { type: "string" },
    "right-click": { type: "string" },
    hover: { type: "string" },
    clip: { type: "string" },
    wait: { type: "string", default: "2000" },
    viewport: { type: "string", default: "1280x720" },
    outdir: { type: "string", default: "test-results/screenshots" },
    messages: { type: "string" },
    "local-storage": { type: "string", multiple: true, default: [] },
    "update-ready": { type: "boolean", default: false },
  },
  strict: true,
});

// `--local-storage key=value` (repeatable) seeds a localStorage entry before the
// app mounts, so device-level preferences (thread layout, transcript options, …)
// can be captured in their non-default state.
const localStorageSeeds = args["local-storage"].map((entry) => {
  const separatorIndex = entry.indexOf("=");
  if (separatorIndex < 1) {
    throw new Error(`--local-storage expects key=value, received "${entry}"`);
  }
  return [entry.slice(0, separatorIndex), entry.slice(separatorIndex + 1)];
});

const activeChannel = args["active-channel"];
const rightClick = args["right-click"];

const [vpWidth, vpHeight] = args.viewport.split("x").map(Number);
const waitMs = Number(args.wait);
const outdir = resolve(args.outdir);

if (!existsSync(outdir)) {
  mkdirSync(outdir, { recursive: true });
}

function resolveSelector(value) {
  return value.startsWith("[") ? value : `[data-testid="${value}"]`;
}

function bail(msg) {
  console.error(msg);
  process.exitCode = 1;
  throw new Error(msg);
}

const BASE_URL = "http://127.0.0.1:4173";
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

// `BUZZ_HEADED=1` runs a real browser window instead of headless.
//
// Headless Chromium rasterizes in software (SwiftShader), which mis-renders
// `backdrop-filter` when an ancestor establishes a rounded clip
// (`overflow-hidden` + `border-radius`) — the blurred surface paints as solid
// cyan garbage. The app's blurred chrome (panel headers, composers) hits this
// inside the rounded focus drawer. Headed rendering is correct, as is the real
// app's WKWebView, so this is a capture-only artifact. Default stays headless so
// CI is unaffected.
const browser = await chromium.launch({ headless: !process.env.BUZZ_HEADED });
const page = await browser.newPage({
  viewport: { width: vpWidth, height: vpHeight },
});

// Seed default community (mirrors seedDefaultCommunity in bridge.ts)
await page.addInitScript(() => {
  const communityId = "e2e-default-community";
  const community = {
    id: communityId,
    name: "E2E Test",
    relayUrl: "ws://localhost:3000",
    addedAt: new Date().toISOString(),
  };
  window.localStorage.setItem("buzz-communities", JSON.stringify([community]));
  window.localStorage.setItem("buzz-active-community-id", communityId);
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

// Seed caller-supplied preferences. Must run before the bridge init script:
// React reads persisted state on mount, and the bridge triggers mount.
if (localStorageSeeds.length > 0) {
  await page.addInitScript((seeds) => {
    for (const [key, value] of seeds) {
      window.localStorage.setItem(key, value);
    }
  }, localStorageSeeds);
}

// Install E2E mock bridge config + MockNotification (mirrors installBridge in bridge.ts)
await page.addInitScript(
  ({ updateReady }) => {
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

    window.__BUZZ_E2E__ = {
      mode: "mock",
      ...(updateReady ? { mock: { updateAvailable: true } } : {}),
    };
    window.__BUZZ_E2E_APP_BADGE_COUNT__ = 0;
  },
  { updateReady: args["update-ready"] },
);

try {
  if (args.messages) {
    if (args.route !== "/" && !activeChannel) {
      console.warn("warning: --route is ignored when --messages is provided");
    }

    let messages;
    try {
      messages = JSON.parse(readFileSync(resolve(args.messages), "utf8"));
    } catch (err) {
      bail(`Failed to read messages file: ${err.message}`);
    }

    if (
      !Array.isArray(messages) ||
      messages.length === 0 ||
      messages.some(
        (m) =>
          typeof m.channelName !== "string" || typeof m.content !== "string",
      )
    ) {
      bail(
        "messages file must be a non-empty array of { channelName, content, pubkey?, kind?, mentionPubkeys?, extraTags?, parentEventId? }",
      );
    }

    const targetChannels = new Set(messages.map((m) => m.channelName));

    if (!activeChannel && targetChannels.size > 1) {
      bail(
        "All messages must target the same channelName, or use --active-channel to specify the viewing channel",
      );
    }

    const viewChannel = activeChannel ?? [...targetChannels][0];

    for (const ch of [...targetChannels, viewChannel]) {
      if (!/^[a-z0-9-]+$/.test(ch)) {
        bail(`Invalid channel name: ${ch}`);
      }
    }

    await page.goto(BASE_URL);
    await page.waitForSelector(`[data-testid="channel-${viewChannel}"]`, {
      timeout: 10000,
    });
    await page.click(`[data-testid="channel-${viewChannel}"]`);

    for (const ch of targetChannels) {
      await page.waitForFunction(
        (name) =>
          window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
            channelName: name,
          }) ?? false,
        ch,
        { timeout: 10000 },
      );
    }

    for (const msg of messages) {
      await page.evaluate(
        (m) => {
          window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.(m);
        },
        { ...msg, pubkey: msg.pubkey ?? DEFAULT_MOCK_PUBKEY },
      );
    }

    await page.waitForTimeout(waitMs);
  } else if (activeChannel) {
    if (!/^[a-z0-9-]+$/.test(activeChannel)) {
      bail(`Invalid channel name: ${activeChannel}`);
    }

    await page.goto(BASE_URL);
    await page.waitForSelector(`[data-testid="channel-${activeChannel}"]`, {
      timeout: 10000,
    });
    await page.click(`[data-testid="channel-${activeChannel}"]`);
    await page.waitForTimeout(waitMs);
  } else {
    const url = args.route === "/" ? BASE_URL : `${BASE_URL}/#${args.route}`;
    await page.goto(url);
    await page.waitForTimeout(waitMs);
  }

  if (args.hover) {
    await page.hover(resolveSelector(args.hover));
    await page.waitForTimeout(500);
  }

  if (args.click) {
    await page.click(resolveSelector(args.click));
    await page.waitForTimeout(500);
  } else if (rightClick) {
    await page.click(resolveSelector(rightClick), { button: "right" });
    await page.waitForTimeout(500);
  }

  // Wait for all CSS/Web animations to finish before capturing.
  // Radix components animate in via CSS — without this, screenshots
  // are taken mid-transition and appear greyed-out or partially rendered.
  await page.evaluate(() =>
    Promise.all(document.getAnimations().map((a) => a.finished)),
  );

  const filepath = join(outdir, `${args.name}.png`);
  const clipOpts = args.clip
    ? (() => {
        const [x, y, w, h] = args.clip.split(",").map(Number);
        return { clip: { x, y, width: w, height: h } };
      })()
    : {};
  await page.screenshot({ path: filepath, ...clipOpts });
  console.log(filepath);
} finally {
  await browser.close();
}
