#!/usr/bin/env node
/**
 * Generates the baked nine-slice texture used by Card variant="textured".
 *
 * This file is the source of truth for the procedural visual. It deliberately
 * lives outside runtime code: edit the parameters below, run this script, then
 * visually compare the generated asset before committing it.
 */
import { chromium } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const OUTPUT = path.resolve(
  HERE,
  "../../src/shared/ui/assets/card-texture.png",
);

// CSS-pixel source geometry. Screenshotting at DPR 2 produces a crisp asset.
const CARD_SIZE = 640;
const OUTSET = 96;
const CAPTURE_SIZE = CARD_SIZE + OUTSET * 2;
const DPR = 2;

// Approved texture parameters, archived from the former runtime SVG filter.
const BLUR = 66;
const DILATE = Math.round(BLUR * 0.85);
const THRESHOLD_BIAS = 0.302;
const SLOPE = 8;
const FREQUENCY = 0.999;
const OCTAVES = 3;
const SEED = 5315;

await mkdir(path.dirname(OUTPUT), { recursive: true });

const browser = await chromium.launch();
try {
  const page = await browser.newPage({
    deviceScaleFactor: DPR,
    viewport: { height: CAPTURE_SIZE, width: CAPTURE_SIZE },
  });

  await page.setContent(`<!doctype html>
    <style>
      html, body { margin: 0; width: 100%; height: 100%; background: transparent; }
      #stage { position: relative; width: ${CAPTURE_SIZE}px; height: ${CAPTURE_SIZE}px; }
      #core {
        position: absolute;
        inset: ${OUTSET + BLUR / 2}px;
        background: white;
        filter: blur(${BLUR / 3}px);
      }
    </style>
    <div id="stage">
      <svg width="${CAPTURE_SIZE}" height="${CAPTURE_SIZE}" aria-hidden="true">
        <defs>
          <filter id="texture" x="0" y="0" width="100%" height="100%"
                  filterUnits="userSpaceOnUse" color-interpolation-filters="sRGB">
            <feMorphology in="SourceAlpha" operator="dilate" radius="${DILATE}" result="squared" />
            <feGaussianBlur in="squared" stdDeviation="${BLUR}" result="ramp" />
            <feTurbulence type="fractalNoise" baseFrequency="${FREQUENCY}"
                          numOctaves="${OCTAVES}" seed="${SEED}" result="grain" />
            <feColorMatrix in="grain" result="grainAlpha" type="matrix"
              values="0 0 0 0 0
                      0 0 0 0 0
                      0 0 0 0 0
                      1 0 0 0 0" />
            <feComposite in="ramp" in2="grainAlpha" operator="arithmetic"
                         k1="0" k2="1" k3="-1" k4="${-THRESHOLD_BIAS}" result="dithered" />
            <feComponentTransfer in="dithered" result="specks">
              <feFuncA type="linear" slope="${SLOPE}" intercept="0" />
            </feComponentTransfer>
            <feFlood flood-color="white" result="white" />
            <feComposite in="white" in2="specks" operator="in" />
          </filter>
        </defs>
        <rect x="${OUTSET}" y="${OUTSET}" width="${CARD_SIZE}" height="${CARD_SIZE}"
              fill="white" filter="url(#texture)" />
      </svg>
      <span id="core"></span>
    </div>`);

  await page.locator("#stage").screenshot({
    omitBackground: true,
    path: OUTPUT,
  });
} finally {
  await browser.close();
}

console.log(`Generated ${OUTPUT}`);
console.log(`Asset: ${CAPTURE_SIZE * DPR}×${CAPTURE_SIZE * DPR}px @${DPR}x`);
console.log(`Runtime slice: ${(OUTSET + 112) * DPR}px; outset: ${OUTSET}px`);
