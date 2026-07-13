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
    image.src = svgDataUrl;
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
