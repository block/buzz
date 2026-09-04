import type { ImetaMedia } from "@/features/messages/lib/imetaMediaMarkdown";

type KlipyAsset = {
  height?: number;
  size?: number;
  url?: string;
  width?: number;
};

type KlipyFileSet = {
  gif?: KlipyAsset;
  jpg?: KlipyAsset;
  webp?: KlipyAsset;
};

type KlipyRawGif = {
  file?: {
    hd?: KlipyFileSet;
    md?: KlipyFileSet;
    sm?: KlipyFileSet;
    xs?: KlipyFileSet;
  };
  id?: number;
  slug?: string;
  title?: string;
  type?: string;
};

export type KlipyResponse = {
  data?: {
    data?: KlipyRawGif[];
  };
  result?: boolean;
};

export type KlipyGif = {
  id: number | null;
  original: Required<KlipyAsset>;
  /**
   * Static (non-animated) poster for the GIF, when KLIPY exposes a `jpg`
   * asset. Rendered in place of the animated preview under
   * `prefers-reduced-motion: reduce`.
   */
  poster: Required<KlipyAsset> | null;
  preview: Required<KlipyAsset>;
  slug: string;
  title: string;
};

export type RelayGifSearchInfo = {
  gif?: {
    provider?: string;
    search?: string;
    share?: string;
  };
  supported_extensions?: string[];
};

export type RelayKlipyCapability = {
  searchPath: string;
  sharePath: string;
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

/** The safe relay-relative KLIPY endpoints advertised by NIP-11, if any. */
export function relayKlipyCapability(
  info: RelayGifSearchInfo,
): RelayKlipyCapability | null {
  const searchPath = info.gif?.search;
  const sharePath = info.gif?.share;
  if (
    info.supported_extensions?.includes("buzz-gif") === true &&
    info.gif?.provider === "klipy" &&
    safeRelayPath(searchPath) &&
    safeRelayPath(sharePath)
  ) {
    return { searchPath, sharePath };
  }
  return null;
}

function isCompleteAsset(
  asset: KlipyAsset | undefined,
): asset is Required<KlipyAsset> {
  return (
    typeof asset?.url === "string" &&
    asset.url.length > 0 &&
    typeof asset.width === "number" &&
    typeof asset.height === "number" &&
    typeof asset.size === "number"
  );
}

function firstCompleteAsset(
  ...assets: Array<KlipyAsset | undefined>
): Required<KlipyAsset> | null {
  return assets.find(isCompleteAsset) ?? null;
}

/**
 * Normalize KLIPY's mixed media response to GIF-only results. The API can
 * interleave ad/content records without a file payload; those are intentionally
 * omitted until Buzz has an explicit third-party ad surface.
 */
export function normalizeKlipyGifs(items: KlipyRawGif[]): KlipyGif[] {
  const gifs: KlipyGif[] = [];

  for (const item of items) {
    if (item.type !== "gif" || !item.file || !item.slug) continue;

    const original = firstCompleteAsset(
      item.file.md?.gif,
      item.file.hd?.gif,
      item.file.sm?.gif,
      item.file.xs?.gif,
    );
    const preview = firstCompleteAsset(
      item.file.sm?.webp,
      item.file.sm?.gif,
      item.file.xs?.webp,
      item.file.xs?.gif,
      item.file.md?.webp,
      original ?? undefined,
    );
    if (!original || !preview) continue;

    const poster = firstCompleteAsset(
      item.file.sm?.jpg,
      item.file.xs?.jpg,
      item.file.md?.jpg,
      item.file.hd?.jpg,
    );

    gifs.push({
      id: item.id ?? null,
      original,
      poster,
      preview,
      slug: item.slug,
      title: item.title?.trim() || "GIF",
    });
  }

  return gifs;
}

export function klipyGifFilename(gif: KlipyGif): string {
  const safeSlug = gif.slug
    .toLowerCase()
    .replace(/[^a-z0-9-]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 80);
  return `${safeSlug || "klipy-gif"}.gif`;
}

/**
 * Represent a selected KLIPY GIF as externally hosted media. The empty hash
 * marks it as content-only media: the outgoing builder appends the image URL
 * to the message body but deliberately omits an imeta tag, since Buzz relays
 * only accept verified local `/media/` entries in imeta.
 */
export function klipyGifAttachment(gif: KlipyGif): ImetaMedia {
  return {
    dim: `${gif.original.width}x${gif.original.height}`,
    displayLabel: gif.title,
    filename: klipyGifFilename(gif),
    sha256: "",
    size: gif.original.size,
    type: "image/gif",
    uploaded: 0,
    url: gif.original.url,
  };
}
