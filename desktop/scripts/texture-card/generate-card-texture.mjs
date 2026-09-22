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
const OUTPUT_DIRECTORY = path.resolve(HERE, "../../src/shared/ui/assets");
const DPR = 2;

// Approved texture parameters, archived from the former runtime SVG filter.
const THRESHOLD_BIAS = 0.302;
const SLOPE = 8;
const FREQUENCY = 0.999;
const OCTAVES = 3;
const SEED = 5315;

const TEXTURES = [
  {
    filename: "card-texture.png",
    color: "white",
    cardSize: 640,
    outset: 96,
    blur: 66,
    innerBand: 112,
  },
  {
    filename: "card-texture-dark.png",
    color: "#171b21",
    cardSize: 640,
    outset: 96,
    blur: 66,
    innerBand: 112,
  },
  {
    filename: "card-texture-compact.png",
    color: "white",
    cardSize: 320,
    outset: 24,
    blur: 24,
    innerBand: 44,
  },
  {
    filename: "card-texture-dark-compact.png",
    color: "#171b21",
    cardSize: 320,
    outset: 24,
    blur: 24,
    innerBand: 44,
  },
];

await mkdir(OUTPUT_DIRECTORY, { recursive: true });

const browser = await chromium.launch();
try {
  for (const texture of TEXTURES) {
    const captureSize = texture.cardSize + texture.outset * 2;
    const dilate = Math.round(texture.blur * 0.85);
    const output = path.join(OUTPUT_DIRECTORY, texture.filename);
    const page = await browser.newPage({
      deviceScaleFactor: DPR,
      viewport: { height: captureSize, width: captureSize },
    });

    await page.setContent(`<!doctype html>
      <style>
        html, body { margin: 0; width: 100%; height: 100%; background: transparent; }
        #stage { position: relative; width: ${captureSize}px; height: ${captureSize}px; }
        #core {
          position: absolute;
          inset: ${texture.outset + texture.blur / 2}px;
          background: ${texture.color};
          filter: blur(${texture.blur / 3}px);
        }
      </style>
      <div id="stage">
        <svg width="${captureSize}" height="${captureSize}" aria-hidden="true">
          <defs>
            <filter id="texture" x="0" y="0" width="100%" height="100%"
                    filterUnits="userSpaceOnUse" color-interpolation-filters="sRGB">
              <feMorphology in="SourceAlpha" operator="dilate" radius="${dilate}" result="squared" />
              <feGaussianBlur in="squared" stdDeviation="${texture.blur}" result="ramp" />
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
              <feFlood flood-color="${texture.color}" result="surfaceColor" />
              <feComposite in="surfaceColor" in2="specks" operator="in" />
            </filter>
          </defs>
          <rect x="${texture.outset}" y="${texture.outset}" width="${texture.cardSize}" height="${texture.cardSize}"
                fill="${texture.color}" filter="url(#texture)" />
        </svg>
        <span id="core"></span>
      </div>`);

    await page.locator("#stage").screenshot({
      omitBackground: true,
      path: output,
    });
    await page.close();

    console.log(`Generated ${output}`);
    console.log(`Asset: ${captureSize * DPR}×${captureSize * DPR}px @${DPR}x`);
    console.log(
      `Runtime slice: ${(texture.outset + texture.innerBand) * DPR}px; outset: ${texture.outset}px`,
    );
  }
} finally {
  await browser.close();
}
