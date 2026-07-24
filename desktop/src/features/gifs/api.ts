const KLIPY_API_ROOT = "https://api.klipy.com/api/v1";
const KLIPY_CUSTOMER_ID_STORAGE_KEY = "buzz:klipy-customer-id:v1";

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

type KlipyResponse = {
  data?: {
    data?: KlipyRawGif[];
  };
  errors?: {
    message?: string[];
  };
  result?: boolean;
};

export type KlipyGif = {
  id: number;
  original: Required<KlipyAsset>;
  preview: Required<KlipyAsset>;
  slug: string;
  title: string;
};

function configuredApiKey(): string {
  return import.meta.env?.VITE_KLIPY_API_KEY?.trim() ?? "";
}

export function isKlipyConfigured(): boolean {
  return configuredApiKey().length > 0;
}

function customerId(): string {
  if (typeof window === "undefined") return "buzz-desktop";

  try {
    const existing = window.localStorage.getItem(KLIPY_CUSTOMER_ID_STORAGE_KEY);
    if (existing) return existing;

    const created = globalThis.crypto.randomUUID();
    window.localStorage.setItem(KLIPY_CUSTOMER_ID_STORAGE_KEY, created);
    return created;
  } catch {
    return "buzz-desktop";
  }
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

    gifs.push({
      id: item.id ?? gifs.length,
      original,
      preview,
      slug: item.slug,
      title: item.title?.trim() || "GIF",
    });
  }

  return gifs;
}

function apiUrl(path: string): URL {
  const apiKey = configuredApiKey();
  if (!apiKey) {
    throw new Error("KLIPY is not configured for this build");
  }
  return new URL(`${KLIPY_API_ROOT}/${encodeURIComponent(apiKey)}/${path}`);
}

function responseError(response: KlipyResponse, status: number): Error {
  const message = response.errors?.message?.filter(Boolean).join(" ");
  return new Error(message || `KLIPY request failed (${status})`);
}

export async function fetchKlipyGifs(
  query: string,
  signal?: AbortSignal,
): Promise<KlipyGif[]> {
  const normalizedQuery = query.trim();
  const path = normalizedQuery ? "gifs/search" : "gifs/trending";
  const url = apiUrl(path);
  url.searchParams.set("page", "1");
  url.searchParams.set("per_page", "24");
  url.searchParams.set("customer_id", customerId());
  url.searchParams.set("locale", navigator.language || "en-US");
  if (normalizedQuery) url.searchParams.set("q", normalizedQuery);

  const httpResponse = await fetch(url, { signal });
  const response = (await httpResponse.json()) as KlipyResponse;
  if (!httpResponse.ok || response.result === false) {
    throw responseError(response, httpResponse.status);
  }

  return normalizeKlipyGifs(response.data?.data ?? []);
}

export function klipyGifFilename(gif: KlipyGif): string {
  const safeSlug = gif.slug
    .toLowerCase()
    .replace(/[^a-z0-9-]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 80);
  return `${safeSlug || "klipy-gif"}.gif`;
}

export async function downloadKlipyGif(gif: KlipyGif): Promise<File> {
  const response = await fetch(gif.original.url);
  if (!response.ok) {
    throw new Error(`Could not download GIF (${response.status})`);
  }

  const blob = await response.blob();
  return new File([blob], klipyGifFilename(gif), {
    type: blob.type || "image/gif",
  });
}

/** Record the provider's share event after the user chooses a GIF. */
export async function trackKlipyGifShare(slug: string): Promise<void> {
  const response = await fetch(
    apiUrl(`gifs/share/${encodeURIComponent(slug)}`),
    {
      body: JSON.stringify({ customer_id: customerId() }),
      headers: { "content-type": "application/json" },
      method: "POST",
    },
  );
  if (!response.ok) {
    throw new Error(`Could not record KLIPY share (${response.status})`);
  }
}
