import type { ImetaMedia } from "@/features/messages/lib/imetaMediaMarkdown";

/**
 * One GIPHY rendition. GIPHY encodes every dimension and size as a string
 * (for example `"320"`), so these fields are typed loosely and parsed later.
 */
type GiphyRendition = {
  height?: unknown;
  size?: unknown;
  url?: unknown;
  webp?: unknown;
  webp_size?: unknown;
  width?: unknown;
};

type GiphyImages = {
  downsized?: GiphyRendition;
  downsized_still?: GiphyRendition;
  fixed_width?: GiphyRendition;
  fixed_width_small?: GiphyRendition;
  fixed_width_small_still?: GiphyRendition;
  fixed_width_still?: GiphyRendition;
  original?: GiphyRendition;
  original_still?: GiphyRendition;
};

type GiphyRawGif = {
  alt_text?: unknown;
  id?: unknown;
  images?: GiphyImages;
  slug?: unknown;
  title?: unknown;
  type?: unknown;
};

/**
 * The relay forwards GIPHY's `data` array and drops `meta` and `pagination`.
 * Array entries are untrusted: the relay forwards them as GIPHY sent them,
 * so an entry may be `null` or a non-object.
 */
export type GiphyResponse = {
  data?: unknown[];
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export type GiphyAsset = {
  height: number;
  size: number;
  url: string;
  width: number;
};

export type GiphyImage = {
  height: number;
  url: string;
  width: number;
};

export type GiphyGif = {
  id: string;
  original: GiphyAsset;
  /**
   * Static (non-animated) poster for the GIF, when GIPHY exposes a `*_still`
   * rendition. Rendered in place of the animated preview under
   * `prefers-reduced-motion: reduce`.
   */
  poster: GiphyImage | null;
  preview: GiphyImage;
  slug: string;
  title: string;
};

export type RelayGifSearchInfo = {
  gif?: {
    provider?: string;
    search?: string;
  };
  supported_extensions?: string[];
};

export type RelayGiphyCapability = {
  searchPath: string;
};

function safeRelayPath(path: unknown): path is string {
  return (
    typeof path === "string" &&
    path.startsWith("/") &&
    !path.startsWith("//") &&
    !path.includes("\\") &&
    !path.includes("%") &&
    !path.includes("?") &&
    !path.includes("#") &&
    !path.split("/").some((segment) => segment === "." || segment === "..")
  );
}

/** The safe relay-relative GIPHY search endpoint advertised by NIP-11, if any. */
export function relayGiphyCapability(
  info: RelayGifSearchInfo,
): RelayGiphyCapability | null {
  const searchPath = info.gif?.search;
  if (
    info.supported_extensions?.includes("buzz-gif") === true &&
    info.gif?.provider === "giphy" &&
    safeRelayPath(searchPath)
  ) {
    return { searchPath };
  }
  return null;
}

/** Parse a GIPHY string dimension such as `"320"` to an integer, or null. */
function parseIntegerField(value: unknown): number | null {
  if (typeof value !== "string" || !/^\d+$/.test(value)) return null;
  const parsed = Number.parseInt(value, 10);
  return Number.isSafeInteger(parsed) ? parsed : null;
}

function nonEmptyString(value: unknown): value is string {
  return typeof value === "string" && value.length > 0;
}

/** A rendition image with a URL and positive integer dimensions, or null. */
function completeImage(
  rendition: GiphyRendition | undefined,
  url: unknown = rendition?.url,
): GiphyImage | null {
  if (!rendition || !nonEmptyString(url)) return null;
  const width = parseIntegerField(rendition.width);
  const height = parseIntegerField(rendition.height);
  if (width === null || height === null || width <= 0 || height <= 0) {
    return null;
  }
  return { height, url, width };
}

/** A sendable rendition: a complete image that also carries a byte size. */
function completeAsset(
  rendition: GiphyRendition | undefined,
): GiphyAsset | null {
  const image = completeImage(rendition);
  if (!image) return null;
  const size = parseIntegerField(rendition?.size);
  if (size === null || size < 0) return null;
  return { ...image, size };
}

function firstImage(
  ...candidates: Array<GiphyImage | null>
): GiphyImage | null {
  return candidates.find((candidate) => candidate !== null) ?? null;
}

/**
 * Normalize GIPHY search results to the GIF shape the picker renders. GIPHY
 * dimensions arrive as strings, and some GIFs lack some renditions, so each
 * rendition is validated and the first usable one in a fixed order wins.
 * Items without a sendable asset or an animated preview are omitted, as are
 * entries that are not objects (for example `null`).
 */
export function normalizeGiphyGifs(items: unknown[]): GiphyGif[] {
  const gifs: GiphyGif[] = [];

  for (const entry of items) {
    if (!isRecord(entry)) continue;
    const item = entry as GiphyRawGif;
    if (
      item.type !== "gif" ||
      !nonEmptyString(item.id) ||
      !nonEmptyString(item.slug) ||
      typeof item.images !== "object" ||
      item.images === null
    ) {
      continue;
    }
    const { images } = item;

    const original =
      completeAsset(images.downsized) ??
      completeAsset(images.original) ??
      completeAsset(images.fixed_width);
    if (!original) continue;

    const preview = firstImage(
      completeImage(images.fixed_width, images.fixed_width?.webp),
      completeImage(images.fixed_width),
      completeImage(images.fixed_width_small, images.fixed_width_small?.webp),
      completeImage(images.fixed_width_small),
      { height: original.height, url: original.url, width: original.width },
    );
    if (!preview) continue;

    const poster = firstImage(
      completeImage(images.fixed_width_still),
      completeImage(images.fixed_width_small_still),
      completeImage(images.downsized_still),
      completeImage(images.original_still),
    );

    const title = typeof item.title === "string" ? item.title.trim() : "";
    const altText =
      typeof item.alt_text === "string" ? item.alt_text.trim() : "";

    gifs.push({
      id: item.id,
      original,
      poster,
      preview,
      slug: item.slug,
      title: title || altText || "GIF",
    });
  }

  return gifs;
}

export function giphyGifFilename(gif: GiphyGif): string {
  const safeSlug = gif.slug
    .toLowerCase()
    .replace(/[^a-z0-9-]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 80);
  return `${safeSlug || "giphy-gif"}.gif`;
}

/**
 * Represent a selected GIPHY GIF as externally hosted media. The empty hash
 * marks it as content-only media: the outgoing builder appends the image URL
 * to the message body but deliberately omits an imeta tag, since Buzz relays
 * only accept verified local `/media/` entries in imeta.
 */
export function giphyGifAttachment(gif: GiphyGif): ImetaMedia {
  return {
    dim: `${gif.original.width}x${gif.original.height}`,
    displayLabel: gif.title,
    filename: giphyGifFilename(gif),
    sha256: "",
    size: gif.original.size,
    type: "image/gif",
    uploaded: 0,
    url: gif.original.url,
  };
}
