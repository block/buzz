import * as React from "react";
import { AlertCircle, Download, Loader2, X } from "lucide-react";
import { motion, useReducedMotion } from "motion/react";
import { toast } from "sonner";

import {
  formatVoiceNoteDuration,
  isVoiceNoteAttachment,
  nextVoiceNotePlaybackRate,
  resolveAudioAttachment,
  summarizeWaveform,
  voiceNoteBarHeight,
  waveformPeaks,
  type AudioAttachmentImetaEntry,
} from "@/features/messages/lib/audioAttachment";
import { scheduleAudioMediaLoad } from "@/features/messages/lib/audioMediaLoadScheduler";
import { invokeTauri } from "@/shared/api/tauri";
import { fetchMediaBytes } from "@/shared/api/tauriMedia";
import { cn } from "@/shared/lib/cn";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import {
  Attachment,
  AttachmentAction,
  AttachmentActions,
  AttachmentContent,
  AttachmentMedia,
  AttachmentTitle,
} from "@/shared/ui/attachment";
import { useSmoothCorners } from "@/shared/ui/smoothCorners";
import { MorphingPlayPauseIcon } from "./MorphingPlayPauseIcon";

const PLAY_EVENT = "buzz-voice-note-play";
const INITIAL_BAR_COUNT = 38;
const BAR_KEYS = Array.from(
  { length: 256 },
  (_, index) => `voice-note-bar-${index}`,
);

function dotPeaks(count: number): number[] {
  return Array.from({ length: count }, () => 0);
}

function playbackRateLabel(rate: number): string {
  return `${rate === 0.5 ? ".5" : rate}×`;
}

export function renderAudioMessageAttachment(
  entry: AudioAttachmentImetaEntry | undefined,
  href: string | undefined,
  label: string,
  downloadUrl?: string,
) {
  const attachment = resolveAudioAttachment(entry, href, label);
  return attachment ? (
    <AudioMessageAttachment
      {...attachment}
      downloadUrl={isVoiceNoteAttachment(entry) ? undefined : downloadUrl}
    />
  ) : null;
}

function audioMimeForUrl(url: string): string {
  const pathname = url.split("?", 1)[0]?.toLowerCase() ?? "";
  if (pathname.endsWith(".mp4")) return "audio/mp4";
  if (pathname.endsWith(".mp3")) return "audio/mpeg";
  if (pathname.endsWith(".ogg")) return "audio/ogg";
  return "audio/wav";
}

function isAbortError(error: unknown): boolean {
  return error instanceof DOMException && error.name === "AbortError";
}

async function decodeSamples(
  url: string,
  signal: AbortSignal,
): Promise<Float32Array> {
  const response = await fetch(url, { signal });
  if (!response.ok) throw new Error(`Audio fetch failed (${response.status})`);
  const bytes = await response.arrayBuffer();
  if (signal.aborted) {
    throw new DOMException("Audio decode cancelled", "AbortError");
  }
  const context = new AudioContext();
  let rejectCancellation: ((reason?: unknown) => void) | undefined;
  const cancellation = new Promise<never>((_resolve, reject) => {
    rejectCancellation = reject;
  });
  const onAbort = () => {
    void context.close().catch(() => undefined);
    rejectCancellation?.(
      new DOMException("Audio decode cancelled", "AbortError"),
    );
  };
  signal.addEventListener("abort", onAbort, { once: true });
  try {
    const buffer = await Promise.race([
      context.decodeAudioData(bytes),
      cancellation,
    ]);
    if (signal.aborted) {
      throw new DOMException("Audio decode cancelled", "AbortError");
    }
    return buffer.getChannelData(0);
  } finally {
    signal.removeEventListener("abort", onAbort);
    await context.close().catch(() => undefined);
  }
}

export function AudioMessageAttachment({
  composer = false,
  duration: taggedDuration,
  downloadUrl,
  filename,
  href,
  onRemove,
}: {
  composer?: boolean;
  duration?: number;
  downloadUrl?: string;
  filename: string;
  href: string;
  onRemove?: () => void;
}) {
  const audioRef = React.useRef<HTMLAudioElement | null>(null);
  const playbackId = React.useId();
  const mediaRef = React.useRef<HTMLDivElement | null>(null);
  const playbackRateRef = React.useRef<HTMLButtonElement | null>(null);
  const waveformRef = React.useRef<HTMLDivElement | null>(null);
  const progressWaveformRef = React.useRef<HTMLDivElement | null>(null);
  const progressFrameRef = React.useRef<number | null>(null);
  const shouldReduceMotion = useReducedMotion();
  const [playbackHref, setPlaybackHref] = React.useState<string | undefined>(
    composer || href.startsWith("blob:") || href.startsWith("data:")
      ? href
      : undefined,
  );
  const [loadRequest, setLoadRequest] = React.useState<
    { attempt: number; href: string } | undefined
  >(
    composer || href.startsWith("blob:") || href.startsWith("data:")
      ? { attempt: 0, href }
      : undefined,
  );
  const [barCount, setBarCount] = React.useState(INITIAL_BAR_COUNT);
  const [duration, setDuration] = React.useState(taggedDuration ?? 0);
  const [currentTime, setCurrentTime] = React.useState(0);
  const [isPlaying, setIsPlaying] = React.useState(false);
  const [playbackRate, setPlaybackRate] = React.useState(1);
  const [playbackError, setPlaybackError] = React.useState(false);
  const [waveformError, setWaveformError] = React.useState(false);
  // A Play click before the source is fetched is remembered here so playback
  // starts automatically once loading resolves, instead of silently no-opping.
  const [pendingPlay, setPendingPlay] = React.useState(false);
  const [waveformSummary, setWaveformSummary] = React.useState<
    Float32Array | undefined
  >();
  const [peaks, setPeaks] = React.useState(() => dotPeaks(INITIAL_BAR_COUNT));
  const [waveformReady, setWaveformReady] = React.useState(false);
  useSmoothCorners(mediaRef);
  useSmoothCorners(playbackRateRef);

  React.useEffect(() => {
    const localHref =
      composer || href.startsWith("blob:") || href.startsWith("data:");
    if (localHref) {
      setLoadRequest({ attempt: 0, href });
      return;
    }

    setLoadRequest(undefined);
    const waveform = waveformRef.current;
    if (!waveform || typeof IntersectionObserver === "undefined") {
      setLoadRequest({ attempt: 0, href });
      return;
    }

    const observer = new IntersectionObserver(
      (entries) => {
        if (!entries.some((entry) => entry.isIntersecting)) return;
        setLoadRequest({ attempt: 0, href });
        observer.disconnect();
      },
      { rootMargin: "240px 0px" },
    );
    observer.observe(waveform);
    return () => observer.disconnect();
  }, [composer, href]);

  React.useEffect(() => {
    if (loadRequest?.href !== href) {
      setPlaybackHref(undefined);
      return;
    }
    if (composer || href.startsWith("blob:") || href.startsWith("data:")) {
      setPlaybackHref(href);
      return;
    }

    let active = true;
    let objectUrl: string | undefined;
    setPlaybackHref(undefined);
    const load = scheduleAudioMediaLoad((signal) =>
      fetchMediaBytes(href, signal),
    );
    void load.promise
      .then((bytes) => {
        if (!active) return;
        objectUrl = URL.createObjectURL(
          new Blob([bytes], { type: audioMimeForUrl(href) }),
        );
        setPlaybackHref(objectUrl);
      })
      .catch((error: unknown) => {
        if (active && !isAbortError(error)) {
          setPlaybackHref(rewriteRelayUrl(href));
        }
      });
    return () => {
      active = false;
      load.cancel();
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [composer, href, loadRequest]);

  React.useEffect(() => {
    const waveform = waveformRef.current;
    if (!waveform) return;
    const updateCount = () => {
      setBarCount(
        Math.min(256, Math.max(1, Math.floor((waveform.clientWidth + 2) / 5))),
      );
    };
    updateCount();
    const observer = new ResizeObserver(updateCount);
    observer.observe(waveform);
    return () => observer.disconnect();
  }, []);

  React.useEffect(() => {
    if (!playbackHref) return;
    let active = true;
    setWaveformReady(false);
    setWaveformError(false);
    setWaveformSummary(undefined);
    const load = scheduleAudioMediaLoad((signal) =>
      decodeSamples(playbackHref, signal),
    );
    void load.promise
      .then((samples) => {
        if (!active) return;
        setWaveformSummary(summarizeWaveform(samples));
      })
      .catch((error: unknown) => {
        if (active && !isAbortError(error)) setWaveformError(true);
      });
    return () => {
      active = false;
      load.cancel();
    };
  }, [playbackHref]);

  React.useEffect(() => {
    setPeaks(
      waveformSummary
        ? waveformPeaks(waveformSummary, barCount)
        : dotPeaks(barCount),
    );
    if (waveformSummary) setWaveformReady(true);
  }, [barCount, waveformSummary]);

  React.useEffect(() => {
    const handleOtherPlayback = (event: Event) => {
      const detail = (event as CustomEvent<string>).detail;
      if (detail !== playbackId) audioRef.current?.pause();
    };
    window.addEventListener(PLAY_EVENT, handleOtherPlayback);
    return () => window.removeEventListener(PLAY_EVENT, handleOtherPlayback);
  }, [playbackId]);

  const paintProgress = React.useCallback((time: number, knownDuration = 0) => {
    const audio = audioRef.current;
    const progressWaveform = progressWaveformRef.current;
    if (!progressWaveform) return;
    const audioDuration =
      knownDuration > 0
        ? knownDuration
        : audio && Number.isFinite(audio.duration)
          ? audio.duration
          : 0;
    const ratio = audioDuration > 0 ? time / audioDuration : 0;
    const remaining = Math.max(0, Math.min(1, 1 - ratio)) * 100;
    progressWaveform.style.clipPath = `inset(0 ${remaining}% 0 0)`;
  }, []);

  React.useEffect(() => {
    if (!isPlaying) {
      if (progressFrameRef.current !== null) {
        window.cancelAnimationFrame(progressFrameRef.current);
        progressFrameRef.current = null;
      }
      return;
    }

    const paintFrame = () => {
      const audio = audioRef.current;
      if (!audio || audio.paused) {
        progressFrameRef.current = null;
        return;
      }
      paintProgress(audio.currentTime);
      progressFrameRef.current = window.requestAnimationFrame(paintFrame);
    };
    progressFrameRef.current = window.requestAnimationFrame(paintFrame);
    return () => {
      if (progressFrameRef.current !== null) {
        window.cancelAnimationFrame(progressFrameRef.current);
        progressFrameRef.current = null;
      }
    };
  }, [isPlaying, paintProgress]);

  const startPlayback = React.useCallback(() => {
    const audio = audioRef.current;
    if (!audio) return;
    window.dispatchEvent(new CustomEvent(PLAY_EVENT, { detail: playbackId }));
    void audio.play().catch(() => {
      setIsPlaying(false);
      setPlaybackError(true);
    });
  }, [playbackId]);

  const togglePlayback = React.useCallback(() => {
    const audio = audioRef.current;
    if (!audio) return;
    if (!playbackHref) {
      // Source not fetched yet: request the load and remember the intent so
      // playback begins as soon as it arrives, rather than dropping the click.
      setPendingPlay(true);
      setLoadRequest((request) =>
        request?.href === href ? request : { attempt: 0, href },
      );
      return;
    }
    if (audio.paused) {
      startPlayback();
    } else {
      setPendingPlay(false);
      audio.pause();
    }
  }, [href, playbackHref, startPlayback]);

  // Fulfill a Play click that landed before the source finished loading.
  React.useEffect(() => {
    if (!pendingPlay || !playbackHref) return;
    setPendingPlay(false);
    startPlayback();
  }, [pendingPlay, playbackHref, startPlayback]);

  // Drop a pending intent if playback itself fails, so the button leaves its
  // loading state and the user can retry. Waveform decode failure is unrelated
  // to playback and must not cancel the intent.
  React.useEffect(() => {
    if (playbackError) setPendingPlay(false);
  }, [playbackError]);

  const retryPlayback = React.useCallback(() => {
    setPlaybackError(false);
    setWaveformError(false);
    setPlaybackHref(undefined);
    setLoadRequest((request) => ({
      attempt: request?.href === href ? request.attempt + 1 : 0,
      href,
    }));
  }, [href]);

  const timeLabel = isPlaying
    ? formatVoiceNoteDuration(Math.max(0, duration - currentTime))
    : formatVoiceNoteDuration(duration);
  const nextPlaybackRate = nextVoiceNotePlaybackRate(playbackRate);

  const waveformBars = React.useCallback(
    (active: boolean) =>
      peaks.map((peak, index) => (
        <motion.span
          animate={{ height: voiceNoteBarHeight(peak) }}
          aria-hidden="true"
          className={cn(
            "w-[3px] shrink-0",
            active ? "bg-primary" : "bg-muted-foreground/35",
          )}
          initial={false}
          key={BAR_KEYS[index]}
          style={{ borderRadius: "9999px" }}
          transition={
            shouldReduceMotion
              ? { duration: 0 }
              : { duration: 0.24, ease: [0.23, 1, 0.32, 1] }
          }
        />
      )),
    [peaks, shouldReduceMotion],
  );

  return (
    <Attachment
      className={cn(
        "my-1 w-full max-w-[21rem] gap-2.5 px-2.5 py-2",
        composer && "shadow-none",
      )}
      data-testid={
        composer ? "composer-voice-note-card" : "audio-message-attachment"
      }
      size="sm"
    >
      <AttachmentMedia
        ref={mediaRef}
        className="rounded-lg bg-primary text-primary-foreground"
        data-testid="voice-note-playback-control"
      >
        <button
          aria-label={
            playbackError
              ? "Retry voice note"
              : pendingPlay
                ? "Loading voice note"
                : isPlaying
                  ? "Pause voice note"
                  : "Play voice note"
          }
          className="flex h-full w-full items-center justify-center rounded-md focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
          onClick={playbackError ? retryPlayback : togglePlayback}
          type="button"
        >
          {playbackError ? (
            <AlertCircle aria-hidden="true" />
          ) : pendingPlay ? (
            <Loader2 aria-hidden="true" className="animate-spin" />
          ) : (
            <MorphingPlayPauseIcon isPlaying={isPlaying} />
          )}
        </button>
      </AttachmentMedia>
      <AttachmentContent className="min-w-0">
        <AttachmentTitle className="sr-only">{filename}</AttachmentTitle>
        {playbackError ? (
          <div className="text-xs font-medium text-destructive" role="alert">
            Audio unavailable. Retry playback.
          </div>
        ) : (
          <div
            className="relative h-6 overflow-hidden rounded-sm focus-within:ring-2 focus-within:ring-ring focus-within:ring-offset-1"
            data-testid="voice-note-playback-waveform"
            data-waveform-state={
              waveformError ? "error" : waveformReady ? "ready" : "loading"
            }
            ref={waveformRef}
          >
            {waveformError ? (
              <span className="sr-only" role="status">
                Waveform preview unavailable. Playback may still work.
              </span>
            ) : null}
            <div className="flex h-full items-center gap-0.5">
              {waveformBars(false)}
            </div>
            <div
              aria-hidden="true"
              className="pointer-events-none absolute inset-0 flex items-center gap-0.5 will-change-[clip-path]"
              data-testid="voice-note-progress-waveform"
              ref={progressWaveformRef}
              style={{ clipPath: "inset(0 100% 0 0)" }}
            >
              {waveformBars(true)}
            </div>
            <input
              aria-label="Voice note playback position"
              className="absolute inset-0 h-full w-full cursor-pointer opacity-0"
              max={Math.max(duration, 0.01)}
              min="0"
              onInput={(event) => {
                const next = Number(event.currentTarget.value);
                if (audioRef.current && Number.isFinite(next)) {
                  audioRef.current.currentTime = next;
                  setCurrentTime(next);
                  paintProgress(next, Number(event.currentTarget.max));
                }
              }}
              step="0.01"
              type="range"
              value={Math.min(currentTime, Math.max(duration, 0.01))}
            />
          </div>
        )}
      </AttachmentContent>
      <AttachmentActions className="grid min-w-9 place-items-center">
        <span
          aria-hidden={!composer}
          className={cn(
            "pointer-events-none col-start-1 row-start-1 text-xs tabular-nums text-muted-foreground transition-[opacity,transform] duration-150 ease-out motion-reduce:transition-none",
            !composer &&
              "group-hover/attachment:-translate-y-0.5 group-hover/attachment:opacity-0 group-focus-within/attachment:-translate-y-0.5 group-focus-within/attachment:opacity-0",
          )}
        >
          {timeLabel}
        </span>
        {!composer ? (
          <button
            ref={playbackRateRef}
            aria-label={`Playback speed ${playbackRateLabel(playbackRate)}; next ${playbackRateLabel(nextPlaybackRate)}`}
            className="col-start-1 row-start-1 grid rounded-full bg-primary px-2.5 py-0.5 text-2xs font-semibold tabular-nums text-primary-foreground opacity-0 transition-[opacity,transform] duration-150 ease-out active:scale-95 group-hover/attachment:opacity-100 group-focus-within/attachment:opacity-100 focus-visible:opacity-100 motion-reduce:transition-none motion-reduce:active:scale-100"
            data-testid="voice-note-playback-rate"
            onClick={() => {
              const next = nextVoiceNotePlaybackRate(playbackRate);
              setPlaybackRate(next);
              if (audioRef.current) {
                audioRef.current.defaultPlaybackRate = next;
                audioRef.current.playbackRate = next;
              }
            }}
            type="button"
          >
            <span
              aria-hidden="true"
              className="invisible col-start-1 row-start-1"
            >
              1.5×
            </span>
            <span
              className="col-start-1 row-start-1 text-center"
              data-testid="voice-note-playback-rate-value"
            >
              {playbackRateLabel(playbackRate)}
            </span>
          </button>
        ) : null}
      </AttachmentActions>
      {!composer && downloadUrl ? (
        <AttachmentActions>
          <AttachmentAction
            aria-label={`Download ${filename}`}
            onClick={() => {
              invokeTauri("download_file", {
                filename,
                url: downloadUrl,
              }).catch((error: unknown) => {
                toast.error(
                  error instanceof Error ? error.message : "Download failed",
                );
              });
            }}
            title="Download"
            type="button"
          >
            <Download />
          </AttachmentAction>
        </AttachmentActions>
      ) : null}
      {!composer && onRemove ? (
        <AttachmentActions>
          <AttachmentAction
            aria-label="Remove voice note"
            onClick={onRemove}
            title="Remove"
            type="button"
          >
            <X />
          </AttachmentAction>
        </AttachmentActions>
      ) : null}
      {/* biome-ignore lint/a11y/useMediaCaption: voice notes are user-provided audio */}
      <audio
        onDurationChange={(event) => {
          const next = event.currentTarget.duration;
          if (Number.isFinite(next)) {
            setDuration(next);
            paintProgress(event.currentTarget.currentTime, next);
          }
        }}
        onEnded={() => {
          setCurrentTime(0);
          setIsPlaying(false);
          paintProgress(0);
        }}
        onPause={() => setIsPlaying(false)}
        onPlay={() => {
          setPlaybackError(false);
          setIsPlaying(true);
        }}
        onError={() => {
          setIsPlaying(false);
          setPlaybackError(true);
        }}
        onTimeUpdate={(event) =>
          setCurrentTime(event.currentTarget.currentTime)
        }
        preload="metadata"
        ref={audioRef}
        src={playbackHref}
      />
    </Attachment>
  );
}
