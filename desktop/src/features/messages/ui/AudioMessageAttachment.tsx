import * as React from "react";
import { AlertCircle, Download, X } from "lucide-react";
import { motion, useReducedMotion } from "motion/react";
import { toast } from "sonner";

import {
  formatVoiceNoteDuration,
  isTerminalMediaError,
  isVoiceNoteAttachment,
  nextVoiceNotePlaybackRate,
  resolveAudioAttachment,
  shouldReportPlaybackError,
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
  const localSrc =
    composer || href.startsWith("blob:") || href.startsWith("data:");
  // Playback streams from the relay proxy URL the same way video does, so the
  // click handler can call play() inside the user gesture. Fetching the bytes
  // first and playing from the fetch continuation is refused with
  // NotAllowedError by WebKit and Chromium, which cost the user a second or
  // third click. The proxy handles NIP-FI auth and range requests. Resolve it
  // once per attachment: swapping the source mid-playback would restart it.
  const playbackSrc = React.useMemo(
    () => (localSrc ? href : rewriteRelayUrl(href)),
    [href, localSrc],
  );
  // Waveform decoding still needs the full bytes (AudioContext.decodeAudioData
  // cannot read a cross-origin stream), so the byte transfer stays, but it is
  // no longer on the playback path.
  const [waveformSrc, setWaveformSrc] = React.useState<string | undefined>();
  const [loadRequest, setLoadRequest] = React.useState<
    { attempt: number; href: string } | undefined
  >(localSrc ? { attempt: 0, href } : undefined);
  // Buffer the note as soon as the user shows intent to play it (pointer or
  // keyboard focus), so a Play click finds data instead of starting a cold
  // stream. Warming on hover keeps the transfer bounded to the card the user is
  // actually about to use; warming every mounted card would open a stream per
  // note in the channel.
  const [warm, setWarm] = React.useState(localSrc);
  // Track the user's intent rather than the element's `paused` flag. A refused
  // play() can leave a media element unpaused, and reading `paused` then turned
  // the next click into a pause instead of another play attempt.
  const wantsPlayRef = React.useRef(false);
  const [barCount, setBarCount] = React.useState(INITIAL_BAR_COUNT);
  const [duration, setDuration] = React.useState(taggedDuration ?? 0);
  const [currentTime, setCurrentTime] = React.useState(0);
  const [isPlaying, setIsPlaying] = React.useState(false);
  const [playbackRate, setPlaybackRate] = React.useState(1);
  const [playbackError, setPlaybackError] = React.useState(false);
  const [waveformError, setWaveformError] = React.useState(false);
  const [waveformSummary, setWaveformSummary] = React.useState<
    Float32Array | undefined
  >();
  const [peaks, setPeaks] = React.useState(() => dotPeaks(INITIAL_BAR_COUNT));
  const [waveformReady, setWaveformReady] = React.useState(false);
  useSmoothCorners(mediaRef);
  useSmoothCorners(playbackRateRef);

  React.useEffect(() => {
    if (localSrc) {
      setLoadRequest({ attempt: 0, href });
      return;
    }

    setLoadRequest(undefined);
    const waveform = waveformRef.current;
    if (!waveform || typeof IntersectionObserver === "undefined") {
      setWarm(true);
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
  }, [href, localSrc]);

  // Transfer the bytes only to draw the waveform. Playback streams from
  // `playbackSrc` on its own, so this effect no longer gates a Play click.
  React.useEffect(() => {
    if (localSrc) {
      setWaveformSrc(href);
      return;
    }
    if (loadRequest?.href !== href) {
      setWaveformSrc(undefined);
      return;
    }

    let active = true;
    let objectUrl: string | undefined;
    setWaveformSrc(undefined);
    const load = scheduleAudioMediaLoad((signal) =>
      fetchMediaBytes(href, signal),
    );
    void load.promise
      .then((bytes) => {
        if (!active) return;
        objectUrl = URL.createObjectURL(
          new Blob([bytes], { type: audioMimeForUrl(href) }),
        );
        setWaveformSrc(objectUrl);
      })
      .catch((error: unknown) => {
        if (active && !isAbortError(error)) setWaveformError(true);
      });
    return () => {
      active = false;
      load.cancel();
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [href, loadRequest, localSrc]);

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
    if (!waveformSrc) return;
    let active = true;
    setWaveformReady(false);
    setWaveformError(false);
    setWaveformSummary(undefined);
    const load = scheduleAudioMediaLoad((signal) =>
      decodeSamples(waveformSrc, signal),
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
  }, [waveformSrc]);

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
    wantsPlayRef.current = true;
    window.dispatchEvent(new CustomEvent(PLAY_EVENT, { detail: playbackId }));
    void audio.play().catch((error: unknown) => {
      setIsPlaying(false);
      if (shouldReportPlaybackError(error)) {
        wantsPlayRef.current = false;
        setPlaybackError(true);
        return;
      }
      // Refused for lack of a user gesture, or aborted while the stream was
      // still loading. Keep the intent: the next activation, or the `canplay`
      // retry below, resumes playback instead of asking for another click.
    });
  }, [playbackId]);

  const togglePlayback = React.useCallback(() => {
    const audio = audioRef.current;
    if (!audio) return;
    // Base the choice on whether playback actually started, not on the element's
    // `paused` flag: a refused play() can leave that flag false, and a click that
    // then took the pause branch is what made playback need a third press.
    if (isPlaying) {
      wantsPlayRef.current = false;
      audio.pause();
      return;
    }
    // Called straight from the click handler so the gesture is still active:
    // the media element streams the source itself instead of waiting for a
    // byte transfer to resolve.
    startPlayback();
  }, [isPlaying, startPlayback]);

  // Resume a click whose play() was refused or aborted before the stream had
  // enough data. A play() refused for lack of a gesture is harmless here: the
  // rejection is swallowed and the intent stays armed for the next attempt.
  React.useEffect(() => {
    const audio = audioRef.current;
    if (!audio) return;
    const resume = () => {
      if (!wantsPlayRef.current || !audio.paused) return;
      void audio.play().catch(() => undefined);
    };
    audio.addEventListener("canplay", resume);
    audio.addEventListener("canplaythrough", resume);
    return () => {
      audio.removeEventListener("canplay", resume);
      audio.removeEventListener("canplaythrough", resume);
    };
  }, []);

  const retryPlayback = React.useCallback(() => {
    const audio = audioRef.current;
    setPlaybackError(false);
    setWaveformError(false);
    setLoadRequest((request) => ({
      attempt: request?.href === href ? request.attempt + 1 : 0,
      href,
    }));
    // Retry is the user asking for playback again, so reload the element and
    // start it in this same click rather than only re-arming it for another
    // press. A load that is still short of data leaves the intent set for the
    // `canplay` resume above.
    setCurrentTime(0);
    audio?.load();
    startPlayback();
  }, [href, startPlayback]);

  // Buffer ahead once the card is near the viewport. A cold stream is what let
  // the first Play click land while the bytes were still arriving; asking the
  // element to load early means the click starts playback instead of a fetch.
  React.useEffect(() => {
    const audio = audioRef.current;
    if (!audio || !warm) return;
    if (audio.readyState >= HTMLMediaElement.HAVE_FUTURE_DATA) return;
    if (wantsPlayRef.current) return;
    audio.load();
  }, [warm]);

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
      onFocus={() => setWarm(true)}
      onPointerEnter={() => setWarm(true)}
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
          wantsPlayRef.current = false;
          setCurrentTime(0);
          setIsPlaying(false);
          paintProgress(0);
        }}
        onPause={() => {
          wantsPlayRef.current = false;
          setIsPlaying(false);
        }}
        onPlay={() => {
          wantsPlayRef.current = true;
          setPlaybackError(false);
          setIsPlaying(true);
        }}
        onError={() => {
          setIsPlaying(false);
          // A network or aborted load is recoverable and must not claim the
          // attachment is unusable; only decode/source failures are terminal.
          if (!isTerminalMediaError(audioRef.current?.error?.code)) return;
          wantsPlayRef.current = false;
          setPlaybackError(true);
        }}
        onTimeUpdate={(event) =>
          setCurrentTime(event.currentTarget.currentTime)
        }
        preload={warm ? "auto" : "metadata"}
        ref={audioRef}
        src={playbackSrc}
      />
    </Attachment>
  );
}
