import { fetchMediaBytes } from "@/shared/api/tauriMedia";

type SnapshotAvatarPngDependencies = {
  fetchBytes?: (url: string) => Promise<Uint8Array>;
  createCanvas?: () => HTMLCanvasElement;
  createImage?: () => HTMLImageElement;
};

/**
 * Resolve an avatar to PNG data for the image body of a snapshot PNG.
 *
 * The original avatar URL remains in the manifest so imports preserve the
 * editable source; this only supplies a renderable card thumbnail. Relay
 * media fetches are validated by Rust, which rejects external origins.
 */
export async function resolveSnapshotAvatarPng(
  avatarUrl: string | null | undefined,
  dependencies: SnapshotAvatarPngDependencies = {},
): Promise<string | undefined> {
  const url = avatarUrl?.trim();
  if (!url) return undefined;

  if (isSvgDataUrl(url)) {
    return rasterizeSvg(url, dependencies);
  }

  if (!isHttpsUrl(url)) return undefined;

  try {
    // Rust validates same-relay `/media/` URLs before fetching; other origins
    // fail there rather than being fetched by the webview.
    const bytes = await (dependencies.fetchBytes ?? fetchMediaBytes)(url);
    return `data:image/png;base64,${bytesToBase64(bytes)}`;
  } catch {
    return undefined;
  }
}

function isSvgDataUrl(url: string) {
  return /^data:image\/svg\+xml(?:;[^,]*)?,/i.test(url);
}

function isHttpsUrl(url: string) {
  try {
    return new URL(url).protocol === "https:";
  } catch {
    return false;
  }
}

async function rasterizeSvg(
  svgDataUrl: string,
  dependencies: SnapshotAvatarPngDependencies,
): Promise<string | undefined> {
  try {
    const image = (dependencies.createImage ?? (() => new Image()))();
    image.src = squareEmojiAvatarBackground(svgDataUrl);
    await image.decode();

    const canvas = (
      dependencies.createCanvas ?? (() => document.createElement("canvas"))
    )();
    canvas.width = 512;
    canvas.height = 512;
    const context = canvas.getContext("2d");
    if (!context) return undefined;
    context.drawImage(image, 0, 0, canvas.width, canvas.height);
    return canvas.toDataURL("image/png");
  } catch {
    return undefined;
  }
}

/**
 * Emoji avatars use a circular SVG background in profile surfaces. Snapshot
 * attachments already clip artwork to a rounded-square media slot, so remove
 * that source-level circle before rasterizing to let the artwork fill the slot.
 */
function squareEmojiAvatarBackground(svgDataUrl: string) {
  const commaIndex = svgDataUrl.indexOf(",");
  if (
    commaIndex === -1 ||
    svgDataUrl.slice(0, commaIndex).includes(";base64")
  ) {
    return svgDataUrl;
  }

  try {
    const prefix = svgDataUrl.slice(0, commaIndex + 1);
    const svg = decodeURIComponent(svgDataUrl.slice(commaIndex + 1));
    const squaredSvg = svg.replace(
      /(<rect\b[^>]*\bwidth="512"[^>]*\bheight="512"[^>]*?)\s+rx="256"/u,
      "$1",
    );

    return squaredSvg === svg
      ? svgDataUrl
      : `${prefix}${encodeURIComponent(squaredSvg)}`;
  } catch {
    return svgDataUrl;
  }
}

function bytesToBase64(bytes: Uint8Array) {
  let binary = "";
  const chunkSize = 0x8000;
  for (let offset = 0; offset < bytes.length; offset += chunkSize) {
    binary += String.fromCharCode(
      ...bytes.subarray(offset, offset + chunkSize),
    );
  }
  return btoa(binary);
}
