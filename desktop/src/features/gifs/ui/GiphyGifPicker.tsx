import { useQuery } from "@tanstack/react-query";
import { Heart, LoaderCircle, Search } from "lucide-react";
import { useReducedMotion } from "motion/react";
import * as React from "react";

import type { GiphyGif } from "@/features/gifs/api";
import { filterGifFavorites, useGifFavorites } from "@/features/gifs/favorites";
import { fetchGiphyGifs } from "@/features/gifs/relay";
import { cn } from "@/shared/lib/cn";
import { Input } from "@/shared/ui/input";
import { Skeleton } from "@/shared/ui/skeleton";

type GiphyGifPickerProps = {
  onSelect: (gif: GiphyGif) => void;
  relayUrl: string;
  searchPath: string;
};

const LOADING_SKELETONS = [
  "tall-a",
  "short-a",
  "short-b",
  "tall-b",
  "short-c",
  "short-d",
  "tall-c",
  "short-e",
  "short-f",
  "tall-d",
] as const;

export const GiphyGifPicker = React.memo(function GiphyGifPicker({
  onSelect,
  relayUrl,
  searchPath,
}: GiphyGifPickerProps) {
  const [search, setSearch] = React.useState("");
  const [debouncedSearch, setDebouncedSearch] = React.useState("");
  const [showFavorites, setShowFavorites] = React.useState(false);
  const prefersReducedMotion = useReducedMotion() ?? false;
  const { favorites, isFavorite, toggleFavorite } = useGifFavorites();

  React.useEffect(() => {
    const timeout = window.setTimeout(
      () => setDebouncedSearch(search.trim()),
      500,
    );
    return () => window.clearTimeout(timeout);
  }, [search]);

  // Favorites render from local storage, so the GIPHY query pauses while the
  // favorites view is open and resumes with its cache when the user returns.
  const gifsQuery = useQuery({
    enabled: !showFavorites,
    queryFn: ({ signal }) =>
      fetchGiphyGifs(relayUrl, searchPath, debouncedSearch, signal),
    queryKey: ["giphy-gifs", relayUrl, searchPath, debouncedSearch],
    retry: false,
    staleTime: 5 * 60 * 1_000,
  });

  const filteredFavorites = React.useMemo(
    () => (showFavorites ? filterGifFavorites(favorites, search) : []),
    [favorites, search, showFavorites],
  );

  const renderGrid = (gifs: GiphyGif[]) => (
    <div className="columns-2 gap-1.5" data-testid="giphy-gif-grid">
      {gifs.map((gif) => {
        const staticPoster = prefersReducedMotion ? gif.poster : null;
        const showAnimated = !prefersReducedMotion;
        const favorite = isFavorite(gif.id);
        return (
          <div
            className="group relative mb-1.5 break-inside-avoid overflow-hidden rounded-lg bg-muted"
            key={gif.id}
          >
            <button
              aria-label={`Choose ${gif.title}`}
              className="block w-full outline-hidden ring-offset-background transition-[filter,transform] hover:brightness-110 focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 active:scale-[0.98]"
              onClick={() => onSelect(gif)}
              title={gif.title}
              type="button"
            >
              {showAnimated ? (
                <img
                  alt={gif.title}
                  className="block h-auto w-full"
                  height={gif.preview.height}
                  loading="lazy"
                  src={gif.preview.url}
                  width={gif.preview.width}
                />
              ) : staticPoster ? (
                <img
                  alt={gif.title}
                  className="block h-auto w-full"
                  height={staticPoster.height}
                  loading="lazy"
                  src={staticPoster.url}
                  width={staticPoster.width}
                />
              ) : (
                <span
                  className="flex aspect-video w-full items-center justify-center bg-muted px-2 text-center text-xs text-muted-foreground"
                  data-testid="giphy-gif-static-placeholder"
                >
                  {gif.title}
                </span>
              )}
            </button>
            <button
              aria-label={
                favorite
                  ? `Remove ${gif.title} from favorites`
                  : `Add ${gif.title} to favorites`
              }
              aria-pressed={favorite}
              className={cn(
                "absolute right-1 top-1 flex h-7 w-7 items-center justify-center rounded-full bg-background/80 text-foreground shadow-sm outline-hidden backdrop-blur-sm transition-opacity hover:bg-background focus-visible:opacity-100 focus-visible:ring-2 focus-visible:ring-ring",
                favorite ? "opacity-100" : "opacity-0 group-hover:opacity-100",
              )}
              data-testid="giphy-gif-favorite-button"
              onClick={() => toggleFavorite(gif)}
              type="button"
            >
              <Heart
                aria-hidden
                className={cn(
                  "h-4 w-4",
                  favorite && "fill-current text-destructive",
                )}
              />
            </button>
          </div>
        );
      })}
    </div>
  );

  return (
    <div className="flex h-[435px] w-[352px] flex-col bg-muted text-foreground">
      <div className="flex items-center gap-2 p-2.5">
        <div className="relative flex-1">
          <Search
            aria-hidden
            className="pointer-events-none absolute left-2.5 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground"
          />
          <Input
            aria-label={showFavorites ? "Search favorites" : "Search GIPHY"}
            autoFocus
            className="h-9 bg-background/75 pl-8 pr-8 hover:bg-background/85 focus-visible:bg-background focus-visible:ring-ring/40"
            maxLength={50}
            onChange={(event) => setSearch(event.target.value)}
            placeholder={showFavorites ? "Search favorites" : "Search GIPHY"}
            type="search"
            value={search}
          />
          {!showFavorites && gifsQuery.isFetching ? (
            <LoaderCircle
              aria-hidden
              className="absolute right-2.5 top-1/2 h-4 w-4 -translate-y-1/2 animate-spin text-muted-foreground"
            />
          ) : null}
        </div>
        <button
          aria-label={
            showFavorites
              ? "Show GIPHY results"
              : `Show favorites (${favorites.length})`
          }
          aria-pressed={showFavorites}
          className={cn(
            "flex h-9 shrink-0 items-center gap-1 rounded-md px-2.5 text-xs font-medium outline-hidden transition-colors focus-visible:ring-2 focus-visible:ring-ring/40",
            showFavorites
              ? "bg-background text-foreground"
              : "bg-background/75 text-muted-foreground hover:bg-background/85 hover:text-foreground",
          )}
          data-testid="giphy-gif-favorites-toggle"
          onClick={() => setShowFavorites((current) => !current)}
          type="button"
        >
          <Heart
            aria-hidden
            className={cn("h-4 w-4", showFavorites && "fill-current")}
          />
          {favorites.length > 0 ? (
            <span className="tabular-nums">{favorites.length}</span>
          ) : null}
        </button>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto p-2">
        {showFavorites ? (
          filteredFavorites.length === 0 ? (
            <div
              className="flex h-full items-center justify-center px-8 text-center text-sm text-muted-foreground"
              data-testid="giphy-gif-favorites-empty"
            >
              {favorites.length === 0
                ? "No favorite GIFs yet. Use the heart on any GIF to save it here."
                : "No favorites match your search."}
            </div>
          ) : (
            renderGrid(filteredFavorites)
          )
        ) : gifsQuery.isPending ? (
          <div className="grid grid-cols-2 gap-1.5">
            <span className="sr-only">Loading GIFs</span>
            {LOADING_SKELETONS.map((id) => (
              <Skeleton
                className={id.startsWith("tall") ? "h-28" : "h-20"}
                key={id}
              />
            ))}
          </div>
        ) : gifsQuery.isError ? (
          <div className="flex h-full flex-col items-center justify-center gap-3 px-8 text-center">
            <p className="text-sm text-muted-foreground">
              {gifsQuery.error.message}
            </p>
            <button
              className="text-sm font-medium text-primary hover:underline"
              onClick={() => void gifsQuery.refetch()}
              type="button"
            >
              Try again
            </button>
          </div>
        ) : gifsQuery.data.length === 0 ? (
          <div className="flex h-full items-center justify-center px-8 text-center text-sm text-muted-foreground">
            No GIFs found.
          </div>
        ) : (
          renderGrid(gifsQuery.data)
        )}
      </div>

      <div className="border-t border-border/60 px-3 py-2 text-center text-2xs text-muted-foreground">
        Powered by GIPHY
      </div>
    </div>
  );
});
