import * as React from "react";

import type { GiphyAsset, GiphyGif, GiphyImage } from "@/features/gifs/api";

/**
 * Device-local GIF favorites.
 *
 * Favorites live in `localStorage` on this desktop only. They hold the
 * normalized GIF record, so the picker renders them without a GIPHY request
 * and they survive the GIF dropping out of search or trending results.
 * Nothing here is synced through the relay; a cross-device list event is a
 * separate design.
 */
export const GIF_FAVORITES_STORAGE_KEY = "buzz:gif-favorites:v1";
export const MAX_GIF_FAVORITES = 100;

function isPositiveInteger(value: unknown): value is number {
  return typeof value === "number" && Number.isInteger(value) && value > 0;
}

function isNonEmptyString(value: unknown): value is string {
  return typeof value === "string" && value.length > 0;
}

function isHttpsUrl(value: unknown): value is string {
  return isNonEmptyString(value) && value.startsWith("https://");
}

function isGiphyImage(value: unknown): value is GiphyImage {
  if (typeof value !== "object" || value === null) return false;
  const image = value as Record<string, unknown>;
  return (
    isHttpsUrl(image.url) &&
    isPositiveInteger(image.width) &&
    isPositiveInteger(image.height)
  );
}

function isGiphyAsset(value: unknown): value is GiphyAsset {
  if (!isGiphyImage(value)) return false;
  const size = (value as Record<string, unknown>).size;
  return typeof size === "number" && Number.isInteger(size) && size >= 0;
}

/** Accept only records that match the current `GiphyGif` shape exactly. */
export function isStoredGifFavorite(value: unknown): value is GiphyGif {
  if (typeof value !== "object" || value === null) return false;
  const gif = value as Record<string, unknown>;
  return (
    isNonEmptyString(gif.id) &&
    isNonEmptyString(gif.slug) &&
    isNonEmptyString(gif.title) &&
    isGiphyAsset(gif.original) &&
    isGiphyImage(gif.preview) &&
    (gif.poster === null || isGiphyImage(gif.poster))
  );
}

/**
 * Parse the stored favorites list. Malformed JSON, a non-array payload, and
 * entries that do not match the GIF shape are dropped instead of thrown, so a
 * stale or corrupted value can never break the picker. Duplicate ids keep the
 * first occurrence, and the result is capped at `MAX_GIF_FAVORITES`.
 */
export function parseGifFavorites(raw: string | null): GiphyGif[] {
  if (!raw) return [];
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return [];
  }
  if (!Array.isArray(parsed)) return [];

  const seen = new Set<string>();
  const favorites: GiphyGif[] = [];
  for (const entry of parsed) {
    if (!isStoredGifFavorite(entry) || seen.has(entry.id)) continue;
    seen.add(entry.id);
    favorites.push({
      id: entry.id,
      original: {
        height: entry.original.height,
        size: entry.original.size,
        url: entry.original.url,
        width: entry.original.width,
      },
      poster: entry.poster
        ? {
            height: entry.poster.height,
            url: entry.poster.url,
            width: entry.poster.width,
          }
        : null,
      preview: {
        height: entry.preview.height,
        url: entry.preview.url,
        width: entry.preview.width,
      },
      slug: entry.slug,
      title: entry.title,
    });
    if (favorites.length >= MAX_GIF_FAVORITES) break;
  }
  return favorites;
}

export function isGifFavorite(favorites: readonly GiphyGif[], id: string) {
  return favorites.some((favorite) => favorite.id === id);
}

/**
 * Add a GIF to the front of the list, or remove it when already present.
 * The newest favorite renders first. Adding past the cap drops the oldest.
 */
export function toggleGifFavorite(
  favorites: readonly GiphyGif[],
  gif: GiphyGif,
): GiphyGif[] {
  if (isGifFavorite(favorites, gif.id)) {
    return favorites.filter((favorite) => favorite.id !== gif.id);
  }
  return [gif, ...favorites].slice(0, MAX_GIF_FAVORITES);
}

/** Case-insensitive title filter used by the picker's favorites view. */
export function filterGifFavorites(
  favorites: readonly GiphyGif[],
  search: string,
): GiphyGif[] {
  const needle = search.trim().toLowerCase();
  if (!needle) return [...favorites];
  return favorites.filter((favorite) =>
    favorite.title.toLowerCase().includes(needle),
  );
}

function readStoredFavorites(): GiphyGif[] {
  if (typeof window === "undefined") return [];
  try {
    return parseGifFavorites(
      window.localStorage.getItem(GIF_FAVORITES_STORAGE_KEY),
    );
  } catch {
    // Storage can be unavailable in hardened webviews. Favorites then live
    // only for the current session.
    return [];
  }
}

function writeStoredFavorites(favorites: readonly GiphyGif[]) {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(
      GIF_FAVORITES_STORAGE_KEY,
      JSON.stringify(favorites),
    );
  } catch {
    // Same storage caveat as above; the in-memory list still works.
  }
}

export type GifFavorites = {
  favorites: GiphyGif[];
  isFavorite: (id: string) => boolean;
  toggleFavorite: (gif: GiphyGif) => void;
};

/**
 * React access to the device-local favorites list. Writes go to storage
 * immediately, and a `storage` event from another window refreshes the list
 * so two open composers stay in step.
 */
export function useGifFavorites(): GifFavorites {
  const [favorites, setFavorites] =
    React.useState<GiphyGif[]>(readStoredFavorites);

  React.useEffect(() => {
    if (typeof window === "undefined") return;
    const onStorage = (event: StorageEvent) => {
      if (event.key !== null && event.key !== GIF_FAVORITES_STORAGE_KEY) return;
      setFavorites(readStoredFavorites());
    };
    window.addEventListener("storage", onStorage);
    return () => window.removeEventListener("storage", onStorage);
  }, []);

  const toggleFavorite = React.useCallback((gif: GiphyGif) => {
    setFavorites((current) => {
      const next = toggleGifFavorite(current, gif);
      writeStoredFavorites(next);
      return next;
    });
  }, []);

  const isFavorite = React.useCallback(
    (id: string) => isGifFavorite(favorites, id),
    [favorites],
  );

  return { favorites, isFavorite, toggleFavorite };
}
