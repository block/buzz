/**
 * Re-encodes an avatar image to a small square data URL.
 *
 * Sandboxed Project Canvas frames run with `connect-src 'none'` and an
 * `img-src` that excludes remote origins, so an avatar can only reach a widget
 * as a `data:` URL carried inside the RPC payload. That payload has a hard
 * 64 KiB per-message ceiling (`PROJECT_CANVAS_MAX_PORT_MESSAGE_BYTES`) shared
 * by every row in the response, and base64 inflates bytes by a third — so a
 * full-resolution avatar cannot be forwarded verbatim. A 512px gravatar is
 * ~230 KB on the wire and ~307,000 characters once encoded, roughly six times
 * the entire message budget on its own.
 *
 * Decoding it to a square no larger than the display size first turns that into
 * a few thousand characters, which is what makes real avatars deliverable at
 * all. Canvas avatars render between 20px and 42px, so the default 96px target
 * still covers the largest of them on a 2x display.
 */

/** Minimal shape of a decoded image this module can draw. */
export type AvatarImageSource = {
  readonly height: number;
  readonly width: number;
  close?: () => void;
};

/** Minimal square drawing surface the downscaler encodes through. */
export type AvatarRenderTarget = {
  drawImage(
    source: AvatarImageSource,
    sx: number,
    sy: number,
    sw: number,
    sh: number,
    dx: number,
    dy: number,
    dw: number,
    dh: number,
  ): void;
  toDataURL(type: string, quality: number): string;
};

/**
 * Decode/encode seam. Defaults to `createImageBitmap` plus a `<canvas>`;
 * tests substitute fakes because neither exists under Node.
 */
export type AvatarDownscaleDeps = {
  createTarget: (size: number) => AvatarRenderTarget | null;
  decode: (blob: Blob) => Promise<AvatarImageSource | null>;
};

export type AvatarDownscaleOptions = {
  deps?: AvatarDownscaleDeps;
  maxDataUrlLength?: number;
  pixelSize?: number;
};

export const DEFAULT_AVATAR_PIXEL_SIZE = 96;

/**
 * Encoders tried in order. WebP keeps alpha and is the smallest, but a webview
 * that cannot encode it silently returns a PNG from `toDataURL`, so each
 * candidate is verified against the type it asked for and JPEG backs it up.
 */
const ENCODINGS: ReadonlyArray<readonly [string, number]> = [
  ["image/webp", 0.82],
  ["image/jpeg", 0.82],
];

function defaultDeps(): AvatarDownscaleDeps {
  return {
    createTarget: (size) => {
      if (typeof document === "undefined") return null;
      const canvas = document.createElement("canvas");
      canvas.width = size;
      canvas.height = size;
      const context = canvas.getContext("2d");
      if (!context) return null;
      return {
        drawImage: (source, sx, sy, sw, sh, dx, dy, dw, dh) => {
          context.drawImage(
            source as unknown as CanvasImageSource,
            sx,
            sy,
            sw,
            sh,
            dx,
            dy,
            dw,
            dh,
          );
        },
        toDataURL: (type, quality) => canvas.toDataURL(type, quality),
      };
    },
    decode: async (blob) => {
      if (typeof createImageBitmap !== "function") return null;
      return await createImageBitmap(blob);
    },
  };
}

/**
 * Center-crops `source` to a square and re-encodes it at no more than
 * `pixelSize` pixels a side, returning the first encoding that fits
 * `maxDataUrlLength`.
 *
 * Returns null when the image cannot be decoded, the platform offers no
 * drawing surface, or every encoding is still over budget. A null result means
 * "render initials", never a broken image.
 *
 * Never upscales: a source smaller than `pixelSize` is re-encoded at its own
 * size rather than inflated to the target.
 */
export async function downscaleAvatarDataUrl(
  blob: Blob,
  options: AvatarDownscaleOptions = {},
): Promise<string | null> {
  const pixelSize = options.pixelSize ?? DEFAULT_AVATAR_PIXEL_SIZE;
  const maxDataUrlLength = options.maxDataUrlLength ?? Number.POSITIVE_INFINITY;
  const deps = options.deps ?? defaultDeps();

  let source: AvatarImageSource | null = null;
  try {
    source = await deps.decode(blob);
    if (!source || source.width < 1 || source.height < 1) return null;

    const side = Math.min(source.width, source.height);
    const sx = (source.width - side) / 2;
    const sy = (source.height - side) / 2;
    const size = Math.max(1, Math.min(Math.floor(pixelSize), Math.floor(side)));

    const target = deps.createTarget(size);
    if (!target) return null;
    target.drawImage(source, sx, sy, side, side, 0, 0, size, size);

    for (const [type, quality] of ENCODINGS) {
      let encoded: string;
      try {
        encoded = target.toDataURL(type, quality);
      } catch {
        continue;
      }
      // A webview without an encoder for `type` returns some other format
      // instead of failing, so an unrequested prefix means "unsupported".
      if (!encoded.startsWith(`data:${type};base64,`)) continue;
      if (encoded.length <= maxDataUrlLength) return encoded;
    }
    return null;
  } catch {
    return null;
  } finally {
    source?.close?.();
  }
}
