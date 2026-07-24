import { useQuery } from "@tanstack/react-query";
import { LoaderCircle, Search } from "lucide-react";
import * as React from "react";

import {
  fetchKlipyGifs,
  isKlipyConfigured,
  type KlipyGif,
} from "@/features/gifs/api";
import { Input } from "@/shared/ui/input";
import { Skeleton } from "@/shared/ui/skeleton";

type KlipyGifPickerProps = {
  onSelect: (gif: KlipyGif) => void;
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

export const KlipyGifPicker = React.memo(function KlipyGifPicker({
  onSelect,
}: KlipyGifPickerProps) {
  const [search, setSearch] = React.useState("");
  const [debouncedSearch, setDebouncedSearch] = React.useState("");

  React.useEffect(() => {
    const timeout = window.setTimeout(
      () => setDebouncedSearch(search.trim()),
      250,
    );
    return () => window.clearTimeout(timeout);
  }, [search]);

  const configured = isKlipyConfigured();
  const gifsQuery = useQuery({
    enabled: configured,
    queryFn: ({ signal }) => fetchKlipyGifs(debouncedSearch, signal),
    queryKey: ["klipy-gifs", debouncedSearch],
    staleTime: 5 * 60 * 1_000,
  });

  return (
    <div className="flex h-[435px] w-[352px] flex-col bg-popover text-popover-foreground">
      <div className="border-b border-border/60 p-2.5">
        <div className="relative">
          <Search
            aria-hidden
            className="pointer-events-none absolute left-2.5 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground"
          />
          <Input
            aria-label="Search KLIPY"
            autoFocus
            className="h-9 pl-8 pr-8"
            disabled={!configured}
            onChange={(event) => setSearch(event.target.value)}
            placeholder="Search KLIPY"
            type="search"
            value={search}
          />
          {gifsQuery.isFetching ? (
            <LoaderCircle
              aria-hidden
              className="absolute right-2.5 top-1/2 h-4 w-4 -translate-y-1/2 animate-spin text-muted-foreground"
            />
          ) : null}
        </div>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto p-2">
        {!configured ? (
          <div className="flex h-full items-center justify-center px-8 text-center text-sm text-muted-foreground">
            GIF search is not configured for this build.
          </div>
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
          <div className="columns-2 gap-1.5" data-testid="klipy-gif-grid">
            {gifsQuery.data.map((gif) => (
              <button
                aria-label={`Choose ${gif.title}`}
                className="mb-1.5 block w-full break-inside-avoid overflow-hidden rounded-lg bg-muted outline-hidden ring-offset-background transition-[filter,transform] hover:brightness-110 focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 active:scale-[0.98]"
                key={`${gif.id}-${gif.slug}`}
                onClick={() => onSelect(gif)}
                title={gif.title}
                type="button"
              >
                <img
                  alt={gif.title}
                  className="block h-auto w-full"
                  height={gif.preview.height}
                  loading="lazy"
                  src={gif.preview.url}
                  width={gif.preview.width}
                />
              </button>
            ))}
          </div>
        )}
      </div>

      <div className="border-t border-border/60 px-3 py-2 text-center text-2xs text-muted-foreground">
        Powered by KLIPY
      </div>
    </div>
  );
});
