import * as React from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  AlertCircle,
  ArrowUpRight,
  Check,
  Clock3,
  Disc3,
  ImageOff,
  Music2,
  RefreshCw,
  ShieldCheck,
  X,
} from "lucide-react";
import { toast } from "sonner";

import { useIsMobile } from "@/shared/hooks/use-mobile";
import {
  releaseRunViewState,
  type ReleaseRunPayload,
  type ReleaseRunTrack,
} from "@/shared/lib/releaseRunLink";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { Sheet, SheetContent, SheetTrigger } from "@/shared/ui/sheet";

function formatFinishedAt(value: string): string {
  const date = new Date(value);
  return new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  }).format(date);
}

function trackDestination(track: ReleaseRunTrack): string | undefined {
  return track.detailsUrl ?? track.sourceUrl;
}

function ReleaseArtwork({ track }: { track: ReleaseRunTrack }) {
  const [failed, setFailed] = React.useState(false);
  if (!track.artworkUrl || failed) {
    return (
      <div className="flex size-13 shrink-0 items-center justify-center rounded-xl bg-foreground/[0.055] text-muted-foreground ring-1 ring-border/45 dark:bg-white/[0.055]">
        {track.artworkUrl ? (
          <ImageOff aria-hidden="true" className="size-4" />
        ) : (
          <Music2 aria-hidden="true" className="size-4" />
        )}
      </div>
    );
  }

  return (
    <img
      alt=""
      className="size-13 shrink-0 rounded-xl object-cover ring-1 ring-black/10 dark:ring-white/15"
      loading="lazy"
      onError={() => setFailed(true)}
      src={track.artworkUrl}
    />
  );
}

function ReleaseTrackRow({
  onOpen,
  track,
}: {
  onOpen: (href: string) => void;
  track: ReleaseRunTrack;
}) {
  const destination = trackDestination(track);
  const content = (
    <>
      <ReleaseArtwork track={track} />
      <div className="min-w-0 flex-1 self-center">
        <p className="truncate text-xs font-medium text-muted-foreground">
          {track.artist}
        </p>
        <p className="mt-0.5 truncate text-sm font-semibold leading-5 text-foreground">
          {track.title}
          {track.version ? (
            <span className="font-normal text-muted-foreground">
              {` · ${track.version}`}
            </span>
          ) : null}
        </p>
        <p className="mt-1 truncate text-2xs leading-4 text-muted-foreground/80">
          {[track.label, track.releaseDate, track.source]
            .filter(Boolean)
            .join(" · ")}
        </p>
      </div>
      {destination ? (
        <ArrowUpRight
          aria-hidden="true"
          className="size-4 shrink-0 self-center text-muted-foreground/65 transition-colors duration-150 ease-out group-hover/release-track:text-foreground motion-reduce:transition-none"
        />
      ) : (
        <Check
          aria-hidden="true"
          className="size-4 shrink-0 self-center text-emerald-500"
        />
      )}
    </>
  );

  if (!destination) {
    return (
      <div
        className="flex min-h-19 items-start gap-3 px-4 py-3"
        data-release-track={track.id}
      >
        {content}
      </div>
    );
  }

  return (
    <button
      aria-label={`Open ${track.artist} — ${track.title} in Trakd`}
      className="group/release-track flex min-h-19 w-full items-start gap-3 px-4 py-3 text-left transition-colors duration-150 ease-out hover:bg-foreground/[0.045] focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring/70 active:bg-foreground/[0.075] motion-reduce:transition-none dark:hover:bg-white/[0.055] dark:active:bg-white/[0.08]"
      data-release-track={track.id}
      onClick={() => onOpen(destination)}
      type="button"
    >
      {content}
    </button>
  );
}

function ReleaseRunState({ payload }: { payload: ReleaseRunPayload }) {
  const state = releaseRunViewState(payload);
  if (state === "ready") return null;

  const stateContent = {
    empty: {
      icon: Disc3,
      title: "No tracks released",
      description: `${payload.checked} checks completed; ${payload.held} stayed held.`,
    },
    failed: {
      icon: AlertCircle,
      title: "Run needs attention",
      description: payload.sourceHealth,
    },
    loading: {
      icon: RefreshCw,
      title: "Release check in progress",
      description:
        "This preview will be populated by the completed run report.",
    },
  }[state];
  const StateIcon = stateContent.icon;

  return (
    <div className="flex min-h-44 flex-col items-center justify-center px-8 py-10 text-center">
      <span className="flex size-10 items-center justify-center rounded-full bg-foreground/[0.055] text-muted-foreground ring-1 ring-border/45 dark:bg-white/[0.055]">
        <StateIcon
          aria-hidden="true"
          className={`size-4 ${state === "loading" ? "animate-spin motion-reduce:animate-none" : ""}`}
        />
      </span>
      <p className="mt-3 text-sm font-semibold text-foreground">
        {stateContent.title}
      </p>
      <p className="mt-1 max-w-72 text-xs leading-5 text-muted-foreground">
        {stateContent.description}
      </p>
    </div>
  );
}

function ReleaseRunSurface({
  onClose,
  payload,
}: {
  onClose: () => void;
  payload: ReleaseRunPayload;
}) {
  const openTrack = React.useCallback(
    (href: string) => {
      onClose();
      void openUrl(href).catch(() => toast.error("Could not open track"));
    },
    [onClose],
  );
  const state = releaseRunViewState(payload);

  return (
    <section
      aria-label={`${payload.runName} release results`}
      className="buzz-release-run-surface overflow-hidden rounded-[1.375rem] text-foreground"
      data-release-run-preview=""
      data-release-run-state={state}
    >
      <header className="relative border-b border-border/45 px-4 pb-3.5 pt-4 pr-12">
        <div className="flex items-start gap-3">
          <span className="mt-0.5 flex size-9 shrink-0 items-center justify-center rounded-xl bg-[oklch(0.64_0.23_18)] text-white shadow-[inset_0_1px_0_rgb(255_255_255/0.24),0_8px_18px_-10px_oklch(0.64_0.23_18/0.8)]">
            <Disc3 aria-hidden="true" className="size-4" />
          </span>
          <div className="min-w-0">
            <h2 className="truncate text-base font-semibold tracking-tight">
              {payload.released === 0
                ? "Release run"
                : `Released · ${payload.released} ${payload.released === 1 ? "track" : "tracks"}`}
            </h2>
            <div className="mt-1 flex min-w-0 items-center gap-1.5 text-2xs text-muted-foreground">
              <Clock3 aria-hidden="true" className="size-3 shrink-0" />
              <span className="truncate">
                {formatFinishedAt(payload.finishedAt)}
              </span>
              <span aria-hidden="true">·</span>
              <span className="truncate">{payload.runName}</span>
            </div>
          </div>
        </div>
        <button
          aria-label="Close release preview"
          className="absolute right-3 top-3 flex size-8 items-center justify-center rounded-lg text-muted-foreground transition-[background-color,color,scale] duration-150 ease-out hover:bg-foreground/[0.06] hover:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring/70 active:scale-[0.96] motion-reduce:transition-none dark:hover:bg-white/[0.07]"
          data-testid="release-run-close"
          onClick={onClose}
          type="button"
        >
          <X aria-hidden="true" className="size-4" />
        </button>
      </header>

      <div className="max-h-[min(31rem,calc(100vh-12rem))] overflow-y-auto overscroll-contain">
        {state === "ready" ? (
          <div className="divide-y divide-border/45" data-release-run-tracks="">
            {payload.tracks.map((track) => (
              <ReleaseTrackRow
                key={track.id}
                onOpen={openTrack}
                track={track}
              />
            ))}
          </div>
        ) : (
          <ReleaseRunState payload={payload} />
        )}
      </div>

      <footer className="flex items-center gap-2 border-t border-border/45 bg-foreground/[0.025] px-4 py-3 text-2xs leading-4 text-muted-foreground dark:bg-white/[0.025]">
        <ShieldCheck
          aria-hidden="true"
          className="size-3.5 shrink-0 text-emerald-500"
        />
        <span className="min-w-0 truncate">{payload.sourceHealth}</span>
      </footer>
    </section>
  );
}

export function ReleaseRunLink({
  children,
  payload,
}: {
  children: React.ReactNode;
  payload: ReleaseRunPayload;
}) {
  const [open, setOpen] = React.useState(false);
  const isMobile = useIsMobile();
  const trigger = (
    <button
      className="font-medium text-primary underline underline-offset-4 transition-[color,scale] duration-150 ease-out hover:text-primary/80 focus-visible:rounded-sm focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring/70 active:scale-[0.98] motion-reduce:transition-none"
      data-release-run-trigger=""
      type="button"
    >
      {children}
    </button>
  );

  if (isMobile) {
    return (
      <Sheet onOpenChange={setOpen} open={open}>
        <SheetTrigger asChild>{trigger}</SheetTrigger>
        <SheetContent
          className="border-0 bg-transparent p-2 pb-[max(0.5rem,env(safe-area-inset-bottom))] shadow-none data-[state=closed]:duration-150 data-[state=open]:duration-200 [&>button]:hidden"
          data-testid="release-run-sheet"
          side="bottom"
        >
          <ReleaseRunSurface onClose={() => setOpen(false)} payload={payload} />
        </SheetContent>
      </Sheet>
    );
  }

  return (
    <Popover onOpenChange={setOpen} open={open}>
      <PopoverTrigger asChild>{trigger}</PopoverTrigger>
      <PopoverContent
        align="start"
        className="w-[30rem] max-w-[calc(100vw-2rem)] border-0 bg-transparent p-0 shadow-none"
        data-testid="release-run-popover"
        side="bottom"
        sideOffset={8}
      >
        <ReleaseRunSurface onClose={() => setOpen(false)} payload={payload} />
      </PopoverContent>
    </Popover>
  );
}
