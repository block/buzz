import {
  Music,
  Music2,
  Music3,
  Music4,
  Pause,
  Play,
  Plug,
  SkipBack,
  SkipForward,
} from "lucide-react";
import * as React from "react";

import { useUsersBatchQuery } from "@/features/profile/hooks";
import {
  useSpotifyConnectionMutations,
  useSpotifyDevicesQuery,
  useSpotifyPlaybackControlMutations,
  useSpotifyPlaybackMutation,
  useSpotifyPlaybackStateQuery,
  useSpotifyStatusQuery,
} from "@/features/spotify/hooks";
import { relayClient } from "@/shared/api/relayClient";
import { signRelayEvent } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_HUDDLE_SPOTIFY_DJ,
  KIND_HUDDLE_SPOTIFY_DJ_LIVE,
} from "@/shared/constants/kinds";
import { cn } from "@/shared/lib/cn";
import { useNow } from "@/shared/lib/useNow";
import { Button } from "@/shared/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { Spinner } from "@/shared/ui/spinner";
import { Switch } from "@/shared/ui/switch";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

type SpotifyHuddleControlProps = {
  agentPubkeys: string[];
  currentPubkey: string | null;
  isHuddleVisible: boolean;
  participants: string[];
  reactionChannelId: string | null;
  reactionSenderName: string;
};

const HUDDLE_REACTION_NAME_MAX = 48;
const SPOTIFY_FLOATING_NOTE_LIMIT = 7;
const SPOTIFY_OPTIMISTIC_PLAYBACK_MS = 3_000;

type SpotifyFloatingNote = {
  id: number;
  iconIndex: number;
  xRem: number;
  driftRem: number;
  liftRem: number;
  sizeRem: number;
  rotationDeg: number;
  rotationDeltaDeg: number;
  durationMs: number;
};

type SpotifyFloatingNoteStyle = React.CSSProperties & {
  "--buzz-spotify-note-x": string;
  "--buzz-spotify-note-drift": string;
  "--buzz-spotify-note-lift": string;
  "--buzz-spotify-note-size": string;
  "--buzz-spotify-note-rotation": string;
  "--buzz-spotify-note-rotation-delta": string;
  "--buzz-spotify-note-duration": string;
};

type OptimisticSpotifyPlayback = {
  isPlaying: boolean;
  progressMs: number | null;
  updatedAt: number;
};

const SPOTIFY_FLOATING_NOTE_ICONS = [Music, Music2, Music3, Music4] as const;

function firstTagValue(event: RelayEvent, name: string): string | null {
  return event.tags.find((tag) => tag[0] === name)?.[1] ?? null;
}

function clampReactionName(name: string): string {
  const trimmed = name.trim();
  if (trimmed.length <= HUDDLE_REACTION_NAME_MAX) return trimmed;
  return `${trimmed.slice(0, HUDDLE_REACTION_NAME_MAX - 1).trimEnd()}...`;
}

function fallbackNameForPubkey(pubkey?: string | null): string {
  return pubkey ? `Participant ${pubkey.slice(0, 8)}` : "Someone";
}

function normalizePubkey(pubkey?: string | null): string | null {
  const normalized = pubkey?.trim().toLowerCase() ?? "";
  return /^[0-9a-f]{64}$/.test(normalized) ? normalized : null;
}

function randomBetween(min: number, max: number): number {
  return min + Math.random() * (max - min);
}

function createSpotifyFloatingNote(id: number): SpotifyFloatingNote {
  return {
    id,
    iconIndex: Math.floor(Math.random() * SPOTIFY_FLOATING_NOTE_ICONS.length),
    xRem: randomBetween(-0.65, 0.65),
    driftRem: randomBetween(-0.75, 0.75),
    liftRem: randomBetween(1.7, 2.55),
    sizeRem: randomBetween(0.55, 0.95),
    rotationDeg: randomBetween(-28, 28),
    rotationDeltaDeg: randomBetween(-32, 32),
    durationMs: randomBetween(1350, 2050),
  };
}

function parseHuddleSpotifyDjEvent(event: RelayEvent) {
  if (
    event.kind !== KIND_HUDDLE_SPOTIFY_DJ &&
    event.kind !== KIND_HUDDLE_SPOTIFY_DJ_LIVE
  ) {
    return null;
  }

  let contentDjPubkey: string | null = null;
  try {
    const parsed = JSON.parse(event.content) as { dj_pubkey?: unknown };
    contentDjPubkey =
      typeof parsed.dj_pubkey === "string" ? parsed.dj_pubkey : null;
  } catch {
    contentDjPubkey = null;
  }

  const djPubkey = normalizePubkey(
    firstTagValue(event, "dj") ?? contentDjPubkey ?? event.pubkey,
  );
  if (!djPubkey) return null;

  return {
    djPubkey,
    senderName: firstTagValue(event, "sender_name"),
  };
}

function huddleSpotifyDjTags(
  channelId: string,
  djPubkey: string,
  senderName: string,
): string[][] {
  return [
    ["h", channelId],
    ["dj", djPubkey],
    ["sender_name", clampReactionName(senderName)],
  ];
}

function friendlySpotifyPlaybackError(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  const lower = message.toLowerCase();

  if (lower.includes("premium") || lower.includes("403")) {
    return "Spotify playback controls require Spotify Premium.";
  }

  if (
    lower.includes("no_active_device") ||
    lower.includes("no active device") ||
    lower.includes("404")
  ) {
    return "Open Spotify on a device first, then try again.";
  }

  if (lower.includes("restricted") || lower.includes("restriction")) {
    return "Spotify cannot control that device right now.";
  }

  return "Spotify couldn't resume. Open Spotify on a device first, then try again.";
}

function formatSpotifyArtists(artists: string[]): string {
  return artists.length > 0 ? artists.join(", ") : "Unknown artist";
}

function formatSpotifyDuration(ms: number | null | undefined): string {
  if (ms == null || ms <= 0) return "--:--";

  const totalSeconds = Math.floor(ms / 1_000);
  const seconds = totalSeconds % 60;
  const minutes = Math.floor(totalSeconds / 60) % 60;
  const hours = Math.floor(totalSeconds / 3_600);

  if (hours > 0) {
    return `${hours}:${minutes.toString().padStart(2, "0")}:${seconds
      .toString()
      .padStart(2, "0")}`;
  }

  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
}

function spotifyProgressMs({
  durationMs,
  isPlaying,
  nowMs,
  progressMs,
  timestamp,
}: {
  durationMs: number | null | undefined;
  isPlaying: boolean;
  nowMs: number;
  progressMs: number | null | undefined;
  timestamp: number | null | undefined;
}): number {
  const base = progressMs ?? 0;
  const elapsed = isPlaying && timestamp ? Math.max(0, nowMs - timestamp) : 0;
  const next = base + elapsed;
  return durationMs && durationMs > 0 ? Math.min(next, durationMs) : next;
}

function SpotifyFloatingMusicNotes({
  notes,
}: {
  notes: SpotifyFloatingNote[];
}) {
  if (notes.length === 0) return null;

  return (
    <span aria-hidden="true" className="buzz-spotify-floating-notes">
      {notes.map((note) => {
        const NoteIcon = SPOTIFY_FLOATING_NOTE_ICONS[note.iconIndex];
        const style: SpotifyFloatingNoteStyle = {
          "--buzz-spotify-note-x": `${note.xRem}rem`,
          "--buzz-spotify-note-drift": `${note.driftRem}rem`,
          "--buzz-spotify-note-lift": `${note.liftRem}rem`,
          "--buzz-spotify-note-size": `${note.sizeRem}rem`,
          "--buzz-spotify-note-rotation": `${note.rotationDeg}deg`,
          "--buzz-spotify-note-rotation-delta": `${note.rotationDeltaDeg}deg`,
          "--buzz-spotify-note-duration": `${note.durationMs}ms`,
        };

        return (
          <span
            className="buzz-spotify-floating-note"
            key={note.id}
            style={style}
          >
            <NoteIcon className="h-full w-full" />
          </span>
        );
      })}
    </span>
  );
}

export function SpotifyHuddleControl({
  agentPubkeys,
  currentPubkey,
  isHuddleVisible,
  participants,
  reactionChannelId,
  reactionSenderName,
}: SpotifyHuddleControlProps) {
  const spotifyStatusQuery = useSpotifyStatusQuery();
  const spotifyStatus = spotifyStatusQuery.data;
  const spotifyConfigured = Boolean(spotifyStatus?.configured);
  const spotifyConnected = Boolean(spotifyStatus?.connected);
  const spotifyDevicesQuery = useSpotifyDevicesQuery({
    enabled: spotifyConnected,
  });
  const spotifyPlaybackStateQuery = useSpotifyPlaybackStateQuery({
    enabled: spotifyConnected,
  });
  const spotifyConnection = useSpotifyConnectionMutations();
  const spotifyPlayback = useSpotifyPlaybackMutation();
  const spotifyControls = useSpotifyPlaybackControlMutations();
  const participantProfilesQuery = useUsersBatchQuery(participants, {
    enabled: participants.length > 0,
  });
  const spotifyNowMs = useNow(1_000);

  const [isPopoverOpen, setIsPopoverOpen] = React.useState(false);
  const floatingNoteIdRef = React.useRef(0);
  const [floatingNotes, setFloatingNotes] = React.useState<
    SpotifyFloatingNote[]
  >([]);
  const [optimisticPlayback, setOptimisticPlayback] =
    React.useState<OptimisticSpotifyPlayback | null>(null);
  const [djState, setDjState] = React.useState<{
    pubkey: string | null;
    createdAt: number;
  }>({ pubkey: null, createdAt: 0 });
  const [isRequestingDj, setIsRequestingDj] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  const currentPubkeyNormalized = normalizePubkey(currentPubkey);
  const queriedIsPlaying = Boolean(spotifyPlaybackStateQuery.data?.isPlaying);
  const optimisticPlaybackActive = Boolean(
    optimisticPlayback &&
      spotifyNowMs - optimisticPlayback.updatedAt <
        SPOTIFY_OPTIMISTIC_PLAYBACK_MS,
  );
  const effectiveIsPlaying = optimisticPlaybackActive
    ? Boolean(optimisticPlayback?.isPlaying)
    : queriedIsPlaying;
  const isPlayingInHuddle = Boolean(
    isHuddleVisible && spotifyConnected && effectiveIsPlaying,
  );

  React.useEffect(() => {
    if (!optimisticPlayback) return;
    if (spotifyPlaybackStateQuery.data === undefined) return;

    if (queriedIsPlaying === optimisticPlayback.isPlaying) {
      setOptimisticPlayback(null);
    }
  }, [optimisticPlayback, queriedIsPlaying, spotifyPlaybackStateQuery.data]);

  React.useEffect(() => {
    if (!spotifyConnected) {
      setOptimisticPlayback(null);
    }
  }, [spotifyConnected]);

  React.useEffect(() => {
    const prefersReducedMotion = window.matchMedia(
      "(prefers-reduced-motion: reduce)",
    ).matches;
    if (!isPlayingInHuddle || prefersReducedMotion) {
      setFloatingNotes([]);
      return;
    }

    let disposed = false;
    let timeoutId: number | null = null;

    function emitNote() {
      const note = createSpotifyFloatingNote(floatingNoteIdRef.current++);
      setFloatingNotes((current) => [
        ...current.slice(-(SPOTIFY_FLOATING_NOTE_LIMIT - 1)),
        note,
      ]);
      window.setTimeout(() => {
        if (disposed) return;
        setFloatingNotes((current) =>
          current.filter((item) => item.id !== note.id),
        );
      }, note.durationMs + 80);
    }

    function scheduleNext(delayMs: number) {
      timeoutId = window.setTimeout(() => {
        emitNote();
        scheduleNext(randomBetween(420, 780));
      }, delayMs);
    }

    emitNote();
    scheduleNext(randomBetween(420, 780));

    return () => {
      disposed = true;
      if (timeoutId !== null) {
        window.clearTimeout(timeoutId);
      }
    };
  }, [isPlayingInHuddle]);

  React.useEffect(() => {
    if (!reactionChannelId) {
      setDjState({ pubkey: null, createdAt: 0 });
      setIsRequestingDj(false);
      return;
    }

    let disposed = false;
    let cleanup: (() => void) | null = null;

    setDjState({ pubkey: null, createdAt: 0 });
    setIsRequestingDj(false);

    void relayClient
      .subscribeLive(
        {
          kinds: [KIND_HUDDLE_SPOTIFY_DJ, KIND_HUDDLE_SPOTIFY_DJ_LIVE],
          "#h": [reactionChannelId],
          limit: 25,
        },
        (event) => {
          if (disposed) return;

          const djEvent = parseHuddleSpotifyDjEvent(event);
          if (!djEvent) return;

          setDjState((prev) => {
            if (event.created_at < prev.createdAt) return prev;
            return {
              pubkey: djEvent.djPubkey,
              createdAt: event.created_at,
            };
          });
          if (djEvent.djPubkey === currentPubkeyNormalized) {
            setIsRequestingDj(false);
          }
        },
      )
      .then((dispose) => {
        if (disposed) {
          void dispose();
          return;
        }
        cleanup = () => void dispose();
      })
      .catch((subscribeError) => {
        console.error(
          "[huddle] Failed to subscribe to Spotify DJ state:",
          subscribeError,
        );
      });

    return () => {
      disposed = true;
      cleanup?.();
    };
  }, [currentPubkeyNormalized, reactionChannelId]);

  const devices =
    spotifyDevicesQuery.data?.filter(
      (device) => device.id && !device.isRestricted,
    ) ?? [];
  const playbackState = spotifyPlaybackStateQuery.data ?? null;
  const playbackDevice =
    playbackState?.device?.id && !playbackState.device.isRestricted
      ? playbackState.device
      : null;
  const resumeDevice =
    playbackDevice ?? devices.find((device) => device.isActive) ?? devices[0];
  const track = playbackState?.item ?? null;
  const durationMs = track?.durationMs ?? null;
  const queriedProgressMs = spotifyProgressMs({
    durationMs,
    isPlaying: Boolean(playbackState?.isPlaying),
    nowMs: spotifyNowMs,
    progressMs: playbackState?.progressMs,
    timestamp: playbackState?.timestamp,
  });
  const currentProgressMs =
    optimisticPlaybackActive && optimisticPlayback
      ? spotifyProgressMs({
          durationMs,
          isPlaying: optimisticPlayback.isPlaying,
          nowMs: spotifyNowMs,
          progressMs: optimisticPlayback.progressMs ?? queriedProgressMs,
          timestamp: optimisticPlayback.updatedAt,
        })
      : queriedProgressMs;
  const progressPercent =
    durationMs && durationMs > 0
      ? Math.max(0, Math.min(100, (currentProgressMs / durationMs) * 100))
      : 0;
  const controlBusy =
    spotifyPlayback.isPending ||
    spotifyControls.pause.isPending ||
    spotifyControls.next.isPending ||
    spotifyControls.previous.isPending;
  const showButton =
    spotifyConnected ||
    spotifyConfigured ||
    spotifyStatusQuery.isLoading ||
    spotifyStatusQuery.isError;
  const connectBusy =
    spotifyStatusQuery.isLoading || spotifyConnection.connect.isPending;
  const huddleAgentPubkeys = new Set(
    agentPubkeys
      .map(normalizePubkey)
      .filter((pubkey): pubkey is string => pubkey !== null),
  );
  const firstHumanParticipantPubkey =
    participants
      .map(normalizePubkey)
      .find(
        (pubkey): pubkey is string =>
          pubkey !== null && !huddleAgentPubkeys.has(pubkey),
      ) ?? null;
  const djPubkey = djState.pubkey ?? firstHumanParticipantPubkey;
  const isDj = Boolean(
    currentPubkeyNormalized && djPubkey === currentPubkeyNormalized,
  );
  const participantProfiles = participantProfilesQuery.data?.profiles ?? {};
  const djName =
    djPubkey === null
      ? null
      : djPubkey === currentPubkeyNormalized
        ? "You"
        : (participantProfiles[djPubkey]?.displayName ??
          fallbackNameForPubkey(djPubkey));

  async function handlePlaybackControl(
    control: "next" | "play" | "previous" | "pause",
  ) {
    setError(null);

    if (!resumeDevice?.id) {
      setError("Open Spotify on a device first, then try again.");
      return;
    }

    const previousOptimisticPlayback = optimisticPlayback;
    if (control === "play" || control === "pause") {
      setOptimisticPlayback({
        isPlaying: control === "play",
        progressMs: currentProgressMs,
        updatedAt: Date.now(),
      });
    }

    try {
      const input = { deviceId: resumeDevice.id };
      if (control === "play") {
        await spotifyPlayback.mutateAsync(input);
      } else if (control === "pause") {
        await spotifyControls.pause.mutateAsync(input);
      } else if (control === "next") {
        await spotifyControls.next.mutateAsync(input);
      } else {
        await spotifyControls.previous.mutateAsync(input);
      }
    } catch (playbackError) {
      setOptimisticPlayback(previousOptimisticPlayback);
      setError(friendlySpotifyPlaybackError(playbackError));
      console.error("Failed to control Spotify playback:", playbackError);
    } finally {
      void spotifyPlaybackStateQuery.refetch();
    }
  }

  async function handleRequestDj(checked: boolean) {
    if (!checked || isDj || !reactionChannelId || !currentPubkeyNormalized) {
      return;
    }

    const previousDjState = djState;
    const createdAt = Math.floor(Date.now() / 1_000);
    setIsRequestingDj(true);
    setError(null);
    setDjState({ pubkey: currentPubkeyNormalized, createdAt });

    try {
      await relayClient.preconnect();
      const content = JSON.stringify({
        dj_pubkey: currentPubkeyNormalized,
        requested_at: createdAt,
      });
      const tags = huddleSpotifyDjTags(
        reactionChannelId,
        currentPubkeyNormalized,
        reactionSenderName,
      );

      const liveEvent = await signRelayEvent({
        kind: KIND_HUDDLE_SPOTIFY_DJ_LIVE,
        content,
        tags,
      });
      await relayClient.publishEvent(
        liveEvent,
        "Timed out while requesting Spotify DJ controls.",
        "Failed to request Spotify DJ controls.",
      );

      try {
        const storedEvent = await signRelayEvent({
          kind: KIND_HUDDLE_SPOTIFY_DJ,
          content,
          tags,
        });
        await relayClient.publishEvent(
          storedEvent,
          "Timed out while saving Spotify DJ controls.",
          "Failed to save Spotify DJ controls.",
        );
      } catch (storedError) {
        console.warn(
          "[huddle] Spotify DJ controls are live but not persisted:",
          storedError,
        );
      }
    } catch (requestError) {
      setDjState(previousDjState);
      setError("Could not request DJ controls.");
      console.error(
        "[huddle] Failed to request Spotify DJ controls:",
        requestError,
      );
    } finally {
      setIsRequestingDj(false);
    }
  }

  async function handleConnect() {
    setError(null);

    try {
      await spotifyConnection.connect.mutateAsync();
    } catch (connectError) {
      const message =
        connectError instanceof Error
          ? connectError.message
          : String(connectError);
      setError(message || "Could not connect Spotify.");
      console.error("[huddle] Failed to connect Spotify:", connectError);
    }
  }

  if (!showButton) return null;

  return (
    <Popover
      onOpenChange={(open) => {
        setIsPopoverOpen(open);
        if (open && spotifyConnected) {
          void spotifyDevicesQuery.refetch();
          void spotifyPlaybackStateQuery.refetch();
        }
      }}
      open={isPopoverOpen}
    >
      <Tooltip>
        <TooltipTrigger asChild>
          <PopoverTrigger asChild>
            <Button
              aria-label="Spotify controls"
              aria-pressed={isPopoverOpen}
              className={cn(
                "buzz-huddle-control-button relative h-12 w-12 shrink-0 overflow-visible rounded-md",
                isPopoverOpen && "text-foreground",
              )}
              size="icon"
              type="button"
              variant="secondary"
            >
              <Music2 className="h-4 w-4" />
              <SpotifyFloatingMusicNotes notes={floatingNotes} />
            </Button>
          </PopoverTrigger>
        </TooltipTrigger>
        <TooltipContent className="buzz-huddle-tooltip" side="top">
          Spotify controls
        </TooltipContent>
      </Tooltip>
      <PopoverContent
        align="center"
        className="w-80 rounded-xl p-3"
        side="top"
        sideOffset={10}
      >
        {error ? (
          <p
            className="mb-3 rounded-md bg-destructive/10 px-2.5 py-2 text-xs text-destructive"
            role="alert"
          >
            {error}
          </p>
        ) : null}

        {spotifyConnected ? (
          <>
            <div className="flex min-w-0 items-center gap-3">
              {track?.imageUrl ? (
                <img
                  alt=""
                  className="h-14 w-14 shrink-0 rounded-md object-cover"
                  src={track.imageUrl}
                />
              ) : (
                <div className="flex h-14 w-14 shrink-0 items-center justify-center rounded-md bg-muted text-muted-foreground">
                  <Music2 className="h-5 w-5" />
                </div>
              )}
              <div className="min-w-0 flex-1">
                <p className="truncate text-sm font-medium text-foreground">
                  {track?.name ?? "No track playing"}
                </p>
                <p className="mt-0.5 truncate text-xs text-muted-foreground">
                  {track
                    ? formatSpotifyArtists(track.artists)
                    : (resumeDevice?.name ?? "Spotify")}
                </p>
              </div>
            </div>

            <div className="mt-3">
              <div
                aria-label="Spotify playback progress"
                aria-valuemax={durationMs ?? undefined}
                aria-valuemin={0}
                aria-valuenow={
                  durationMs ? Math.round(currentProgressMs) : undefined
                }
                className="h-1.5 overflow-hidden rounded-full bg-muted"
                role="progressbar"
              >
                <div
                  className="h-full rounded-full bg-primary"
                  style={{ width: `${progressPercent}%` }}
                />
              </div>
              <div className="mt-1.5 flex items-center justify-between text-2xs tabular-nums text-muted-foreground">
                <span>{formatSpotifyDuration(currentProgressMs)}</span>
                <span>{formatSpotifyDuration(durationMs)}</span>
              </div>
            </div>

            <div className="mt-3 flex items-center justify-center gap-2">
              {isDj ? (
                <>
                  <Button
                    aria-label="Previous Spotify track"
                    disabled={controlBusy || !resumeDevice?.id}
                    onClick={() => void handlePlaybackControl("previous")}
                    size="icon"
                    type="button"
                    variant="outline"
                  >
                    <SkipBack className="h-4 w-4" />
                  </Button>
                  <Button
                    aria-label={
                      effectiveIsPlaying ? "Pause Spotify" : "Play Spotify"
                    }
                    disabled={!resumeDevice?.id}
                    onClick={() =>
                      void handlePlaybackControl(
                        effectiveIsPlaying ? "pause" : "play",
                      )
                    }
                    size="icon"
                    type="button"
                  >
                    {effectiveIsPlaying ? (
                      <Pause className="h-4 w-4" />
                    ) : (
                      <Play className="h-4 w-4" />
                    )}
                  </Button>
                  <Button
                    aria-label="Next Spotify track"
                    disabled={controlBusy || !resumeDevice?.id}
                    onClick={() => void handlePlaybackControl("next")}
                    size="icon"
                    type="button"
                    variant="outline"
                  >
                    <SkipForward className="h-4 w-4" />
                  </Button>
                </>
              ) : (
                <div className="flex w-full items-center justify-between gap-3 rounded-md border bg-muted/35 px-3 py-2">
                  <div className="min-w-0">
                    <p className="truncate text-xs font-medium text-foreground">
                      {djName ? `${djName} is DJ` : "No DJ"}
                    </p>
                    <p className="truncate text-2xs text-muted-foreground">
                      Request DJ controls
                    </p>
                  </div>
                  <Switch
                    aria-label="Request Spotify DJ controls"
                    checked={isRequestingDj}
                    disabled={isRequestingDj || !currentPubkeyNormalized}
                    onCheckedChange={(checked) => void handleRequestDj(checked)}
                  />
                </div>
              )}
            </div>
          </>
        ) : (
          <div className="flex flex-col gap-3">
            <div className="flex min-w-0 items-center gap-3">
              <div className="flex h-12 w-12 shrink-0 items-center justify-center rounded-md bg-muted text-muted-foreground">
                <Music2 className="h-5 w-5" />
              </div>
              <div className="min-w-0">
                <p className="truncate text-sm font-medium text-foreground">
                  Spotify Premium required
                </p>
                <p className="mt-0.5 text-xs leading-5 text-muted-foreground">
                  Connect a Spotify Premium account.
                </p>
              </div>
            </div>

            <Button
              className="w-full gap-2"
              disabled={connectBusy || !spotifyConfigured}
              onClick={() => void handleConnect()}
              type="button"
            >
              {connectBusy ? (
                <Spinner className="h-4 w-4" />
              ) : (
                <Plug className="h-4 w-4" />
              )}
              Connect Spotify
            </Button>
          </div>
        )}
      </PopoverContent>
    </Popover>
  );
}
