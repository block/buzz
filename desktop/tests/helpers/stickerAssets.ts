import type { Page } from "@playwright/test";

import { mockStickerAssetIndex } from "../../src/testing/stickerFixtures";

/**
 * Matches the URL the app actually requests for a sticker.
 *
 * Both the composer picker and the timeline render `stickerCacheUrl()` — an
 * origin-less `/media/sticker/<author>/<identifier>/<shortcode>/<sha256>` path
 * — which `rewriteRelayUrl()` rewrites onto the mock media proxy port
 * (`http://127.0.0.1:54321/media/…`). The port is matched loosely so a change
 * to the bridge's `MOCK_MEDIA_PROXY_PORT` doesn't silently produce broken
 * images.
 */
const STICKER_ASSET_URL_RE = /^http:\/\/127\.0\.0\.1:\d+\/media\/sticker\//;

/** Stable hue per shortcode so each sticker is visually distinct. */
function hueFor(seed: string): number {
  let hash = 0;
  for (const char of seed) {
    hash = (hash * 31 + (char.codePointAt(0) ?? 0)) % 360;
  }
  return hash;
}

function stickerSvg(emoji: string, shortcode: string): string {
  const hue = hueFor(shortcode);
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 128 128" width="128" height="128">
  <defs>
    <linearGradient id="bg" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="hsl(${hue} 90% 88%)"/>
      <stop offset="1" stop-color="hsl(${(hue + 32) % 360} 85% 74%)"/>
    </linearGradient>
  </defs>
  <rect width="128" height="128" rx="30" fill="url(#bg)"/>
  <rect x="4" y="4" width="120" height="120" rx="26" fill="none" stroke="#ffffff" stroke-width="6"/>
  <circle cx="64" cy="56" r="34" fill="#ffffff" fill-opacity="0.85"/>
  <text x="64" y="72" font-size="42" text-anchor="middle">${emoji}</text>
  <text x="64" y="112" font-size="15" font-family="Helvetica, Arial, sans-serif" font-weight="700" text-anchor="middle" fill="hsl(${hue} 55% 26%)">${shortcode.slice(0, 12)}</text>
</svg>`;
}

/**
 * Serve real pixels for every seeded sticker asset.
 *
 * Without this the `<img>` elements resolve against a media proxy that does
 * not exist in the mock harness, and every sticker renders as a broken image
 * (or, in the timeline, as `StickerMessage`'s "Sticker unavailable" fallback).
 * Unknown sticker paths are aborted rather than fulfilled, so a fixture drift
 * shows up as a visibly missing sticker instead of a generic placeholder.
 */
export async function routeStickerAssets(page: Page): Promise<void> {
  const assets = mockStickerAssetIndex();
  await page.route(STICKER_ASSET_URL_RE, (route) => {
    const key = decodeURIComponent(
      new URL(route.request().url()).pathname.replace("/media/sticker/", ""),
    );
    const asset = assets.get(key);
    if (!asset) return route.abort();
    return route.fulfill({
      contentType: "image/svg+xml",
      body: stickerSvg(asset.emoji, asset.shortcode),
    });
  });
}
