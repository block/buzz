#!/usr/bin/env node
import { spawn } from "node:child_process";
import { mkdir, readFile, stat, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const ARTIFACTORY_BASE =
  "https://global.block-artifacts.com/artifactory/goose-internal/avatars";
const LATEST_URL = `${ARTIFACTORY_BASE}/latest.json`;

const scriptDir = dirname(fileURLToPath(import.meta.url));
const desktopRoot = dirname(scriptDir);
const outputRoot = join(desktopRoot, "src/shared/assets/goose-avatars");
const catalogPath = join(outputRoot, "catalog.json");

const FORMATS = ["webm", "hevc"];

function variantOutputPath(asset, format) {
  const extension = format === "hevc" ? "mp4" : "webm";
  return join(
    outputRoot,
    format,
    asset.collectionId,
    `${asset.id}.${extension}`,
  );
}

function posterOutputPath(asset) {
  return join(outputRoot, "posters", asset.collectionId, `${asset.id}.png`);
}

async function fetchJson(url) {
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`Failed to fetch ${url}: ${response.status}`);
  }
  return response.json();
}

async function fileExistsWithSize(path, byteSize) {
  try {
    const info = await stat(path);
    return info.size === byteSize;
  } catch {
    return false;
  }
}

async function downloadFile(url, path, byteSize) {
  if (await fileExistsWithSize(path, byteSize)) {
    return "skipped";
  }

  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`Failed to download ${url}: ${response.status}`);
  }

  const bytes = new Uint8Array(await response.arrayBuffer());
  if (bytes.byteLength !== byteSize) {
    throw new Error(
      `Downloaded ${url} with ${bytes.byteLength} bytes, expected ${byteSize}.`,
    );
  }

  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, bytes);
  return "downloaded";
}

async function runFfmpeg(args) {
  await new Promise((resolve, reject) => {
    const child = spawn("ffmpeg", args, {
      stdio: ["ignore", "ignore", "pipe"],
    });
    let stderr = "";
    child.stderr.on("data", (chunk) => {
      stderr += chunk.toString();
    });
    child.on("error", reject);
    child.on("close", (code) => {
      if (code === 0) {
        resolve();
      } else {
        reject(
          new Error(
            `ffmpeg exited with ${code}: ${stderr.trim() || "no stderr"}`,
          ),
        );
      }
    });
  });
}

async function ensurePoster(asset) {
  const posterPath = posterOutputPath(asset);
  try {
    await stat(posterPath);
    return "skipped";
  } catch {
    // Generate it below.
  }

  await mkdir(dirname(posterPath), { recursive: true });
  await runFfmpeg([
    "-hide_banner",
    "-loglevel",
    "error",
    "-y",
    "-i",
    variantOutputPath(asset, "webm"),
    "-frames:v",
    "1",
    posterPath,
  ]);
  return "generated";
}

async function main() {
  const latest = await fetchJson(LATEST_URL);
  const manifest = await fetchJson(
    `${ARTIFACTORY_BASE}/${latest.manifestPath}`,
  );
  const versionRoot = `${ARTIFACTORY_BASE}/${manifest.catalogVersion}`;

  await mkdir(outputRoot, { recursive: true });
  await writeFile(catalogPath, `${JSON.stringify(manifest, null, 2)}\n`);

  const totals = {
    downloaded: 0,
    skipped: 0,
    postersGenerated: 0,
    postersSkipped: 0,
  };

  for (const [index, asset] of manifest.assets.entries()) {
    for (const format of FORMATS) {
      const variant = asset.variants[format];
      const sourceUrl = `${versionRoot}/${variant.path}`;
      const result = await downloadFile(
        sourceUrl,
        variantOutputPath(asset, format),
        variant.byteSize,
      );
      totals[result] += 1;
    }

    const posterResult = await ensurePoster(asset);
    if (posterResult === "generated") {
      totals.postersGenerated += 1;
    } else {
      totals.postersSkipped += 1;
    }

    const completed = index + 1;
    if (completed % 5 === 0 || completed === manifest.assets.length) {
      console.log(
        `Synced ${completed}/${manifest.assets.length} Goose avatars...`,
      );
    }
  }

  const catalogBytes = await readFile(catalogPath, "utf8");
  JSON.parse(catalogBytes);

  console.log(
    `Done. Downloaded ${totals.downloaded}, skipped ${totals.skipped}, generated ${totals.postersGenerated} posters, skipped ${totals.postersSkipped} posters.`,
  );
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
