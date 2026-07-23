import * as React from "react";
import { createPortal } from "react-dom";
import {
  AlertCircle,
  Check,
  Maximize2,
  MessageCircle,
  PanelRight,
  Pause,
  Play,
  SmilePlus,
  Volume2,
  VolumeX,
  X,
} from "lucide-react";
import { EmojiPicker } from "@/features/custom-emoji/ui/EmojiPicker";
import { MessageComposer } from "@/features/messages/ui/MessageComposer";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { ChannelType } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Checkbox } from "@/shared/ui/checkbox";
import { MODAL_BACKDROP_BLUR_CLASS } from "@/shared/ui/modalBackdrop";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { useSmoothCorners } from "@/shared/ui/smoothCorners";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import { Spinner } from "./spinner";
import { useNaturalVideoAspectRatio } from "./videoAspectRatio";
import { useVideoContextMenu } from "./useVideoContextMenu";
import {
  getInlinePlaybackPosition,
  getReviewPlaybackPosition,
  isVideoReviewOpen,
  saveInlinePlaybackPosition,
  saveReviewPlaybackPosition,
  setVideoReviewOpen,
} from "./videoPlayerState";

type VideoReviewReaction = {
  emoji: string;
  emojiUrl?: string;
  count: number;
  reactedByCurrentUser?: boolean;
  users: Array<{
    pubkey: string;
    displayName: string;
    avatarUrl: string | null;
  }>;
};

export type VideoReviewComment = {
  id: string;
  author: string;
  avatarUrl?: string | null;
  body: string;
  createdAt: number;
  time: string;
  /** Parent event id — when it points at another comment, this is a reply. */
  parentId?: string | null;
  reactions?: VideoReviewReaction[];
};

export type VideoReviewContext = {
  channelId?: string | null;
  channelName?: string;
  channelType?: ChannelType | null;
  comments: VideoReviewComment[];
  disabled?: boolean;
  isSending?: boolean;
  onSendComment?: (
    content: string,
    mentionPubkeys: string[],
    mediaTags?: string[][],
    /** Reply to this comment instead of the video message itself. */
    parentEventId?: string,
  ) => Promise<void>;
  onToggleCommentReaction?: (
    comment: VideoReviewComment,
    emoji: string,
    remove: boolean,
  ) => Promise<void>;
  profiles?: UserProfileLookup;
  rootEventId?: string;
  title?: string;
};

type VideoPlayerProps = {
  src: string;
  poster?: string;
  aspectRatio?: number;
  durationSeconds?: number;
  reviewKey?: string;
  reviewContext?: VideoReviewContext;
  /**
   * Original relay `/media/` URL for the right-click Download action, distinct
   * from the possibly-proxied `src` the download command's SSRF check rejects.
   * Omitted for non-relay sources, which hide the Download item.
   */
  downloadUrl?: string;
  /** imeta `filename`, used as the save-dialog name. */
  filename?: string;
};

type TimecodedComment = {
  comment: VideoReviewComment;
  seconds: number | null;
  timecode: string | null;
  text: string;
};

const TIMECODE_RE =
  /^\s*\[((?:(?:\d{1,2}:)?\d{1,2}:)?\d{2}(?:\.\d{1,3})?)\]\s*/;
const QUICK_REACTIONS = ["😂", "😍", "😮", "🙌", "👍", "👎"];
const DEFAULT_PLAYBACK_SPEED = 1;
const INLINE_SPEED_CONTROL_MIN_WIDTH = 220;
const PLAYBACK_SPEEDS = [2, 1.75, 1.5, 1.25, 1, 0.75, 0.5, 0.25];
const TIMECODE_ACCENT_CLASS =
  "bg-[hsl(var(--buzz-video-review-accent,var(--primary))/0.15)] text-[hsl(var(--buzz-video-review-accent-foreground,var(--buzz-video-review-accent,var(--primary))))]";
const TIMECODE_ACCENT_HOVER_CLASS =
  "hover:bg-[hsl(var(--buzz-video-review-accent,var(--primary))/0.3)]";

/**
 * Frosted-glass backing layer for floating media controls. The parent must
 * be `relative isolate` so the `-z-10` layer sits behind the control content
 * but above the video underneath.
 */
function GlassSurface({ className }: { className?: string }) {
  return (
    <div
      aria-hidden="true"
      className={cn(
        "pointer-events-none absolute inset-0 -z-10 rounded-[inherit] border border-white/10 bg-black/35 backdrop-blur-xl backdrop-saturate-150",
        className,
      )}
    />
  );
}

function formatTimecode(
  seconds: number,
  options: { fractionalDigits?: number; trimZeroFraction?: boolean } = {},
): string {
  const safeSeconds = Number.isFinite(seconds) ? Math.max(0, seconds) : 0;
  const fractionalDigits = Math.max(0, options.fractionalDigits ?? 0);
  const precisionFactor = 10 ** fractionalDigits;
  const totalTicks =
    fractionalDigits > 0
      ? Math.round(safeSeconds * precisionFactor)
      : Math.floor(safeSeconds);
  const totalSeconds =
    fractionalDigits > 0
      ? Math.floor(totalTicks / precisionFactor)
      : totalTicks;
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const remainingSeconds = totalSeconds % 60;
  const paddedSeconds = String(remainingSeconds).padStart(2, "0");
  const fraction = fractionalDigits > 0 ? totalTicks % precisionFactor : 0;

  const baseTimecode =
    hours > 0
      ? `${hours}:${String(minutes).padStart(2, "0")}:${paddedSeconds}`
      : `${String(minutes).padStart(2, "0")}:${paddedSeconds}`;

  if (
    fractionalDigits <= 0 ||
    (options.trimZeroFraction === true && fraction === 0)
  ) {
    return baseTimecode;
  }

  return `${baseTimecode}.${String(fraction).padStart(fractionalDigits, "0")}`;
}

function formatCommentTimecode(seconds: number): string {
  return formatTimecode(seconds, {
    fractionalDigits: 1,
    trimZeroFraction: true,
  });
}

function formatPlaybackSpeed(speed: number): string {
  return `${speed}x`;
}

function isPlaybackSpeedOption(speed: number): boolean {
  return PLAYBACK_SPEEDS.some((option) => option === speed);
}

function parseTimecode(value: string): number | null {
  const parts = value.split(":").map((part) => Number(part));
  if (parts.some((part) => !Number.isFinite(part) || part < 0)) {
    return null;
  }

  if (parts.length === 2) {
    return parts[0] * 60 + parts[1];
  }

  if (parts.length === 3) {
    return parts[0] * 3600 + parts[1] * 60 + parts[2];
  }

  return null;
}

function parseTimecodedComment(comment: VideoReviewComment): TimecodedComment {
  const match = comment.body.match(TIMECODE_RE);
  if (!match) {
    return {
      comment,
      seconds: null,
      timecode: null,
      text: comment.body.trim(),
    };
  }

  return {
    comment,
    seconds: parseTimecode(match[1]),
    timecode: match[1],
    text: comment.body.slice(match[0].length).trim(),
  };
}

function sortTimecodedComments(
  left: TimecodedComment,
  right: TimecodedComment,
): number {
  if (left.seconds !== null && right.seconds !== null) {
    if (left.seconds !== right.seconds) return left.seconds - right.seconds;
    return left.comment.createdAt - right.comment.createdAt;
  }

  if (left.seconds !== null) return -1;
  if (right.seconds !== null) return 1;
  return left.comment.createdAt - right.comment.createdAt;
}

function fileNameFromUrl(src: string): string {
  const withoutQuery = src.split("?")[0];
  const tail = withoutQuery.split("/").filter(Boolean).pop();
  return tail ? decodeURIComponent(tail) : "Video";
}

function getInlineSurfaceWidth(aspectRatio: number): number {
  return Math.round(Math.min(384, Math.max(160, aspectRatio * 180)));
}

/**
 * Drives a `requestAnimationFrame` loop that mirrors `video.currentTime` into
 * React state while playback is active, so progress bars move smoothly
 * between the browser's coarse `timeupdate` events.
 */
function useSmoothPlaybackTime(
  videoRef: React.RefObject<HTMLVideoElement | null>,
  isActive: boolean,
  onTime: (seconds: number) => void,
) {
  React.useEffect(() => {
    if (!isActive) return;

    let frameId = 0;
    const tick = () => {
      const video = videoRef.current;
      if (video) {
        onTime(video.currentTime);
      }
      frameId = window.requestAnimationFrame(tick);
    };

    frameId = window.requestAnimationFrame(tick);
    return () => window.cancelAnimationFrame(frameId);
  }, [isActive, onTime, videoRef]);
}

/**
 * Serializes seeks against the browser's seek pipeline: while a seek is in
 * flight only the latest requested position is remembered, and it is applied
 * when `seeked` fires. Without this, a slow drag floods the element with
 * seeks and the displayed frame freezes until the drag ends — with it, the
 * video scrubs frame-by-frame under the pointer.
 */
function useThrottledVideoSeek(
  videoRef: React.RefObject<HTMLVideoElement | null>,
) {
  const isSeekingRef = React.useRef(false);
  const queuedSeekRef = React.useRef<number | null>(null);

  const requestSeek = React.useCallback(
    (seconds: number) => {
      const video = videoRef.current;
      if (!video) return;
      // `video.seeking` is the ground truth: if the element is not actually
      // mid-seek, a stale in-flight flag (a seek interrupted by an error, or
      // a skipped `seeked` event) must not wedge the scrubber.
      if (isSeekingRef.current && video.seeking) {
        queuedSeekRef.current = seconds;
        return;
      }
      isSeekingRef.current = true;
      video.currentTime = seconds;
    },
    [videoRef],
  );

  const handleSeeked = React.useCallback(() => {
    isSeekingRef.current = false;
    const queued = queuedSeekRef.current;
    if (queued === null) return;
    queuedSeekRef.current = null;
    const video = videoRef.current;
    if (!video) return;
    isSeekingRef.current = true;
    video.currentTime = queued;
  }, [videoRef]);

  return { handleSeeked, requestSeek };
}

/**
 * Ambient-light glow behind the review video: a small canvas continuously
 * redraws downscaled video frames at ~15fps, then a heavy blur + saturation
 * smears them into a soft colour wash (the same technique as
 * blog.maximeheckel.com's media player glow).
 */
function VideoGlow({
  videoRef,
}: {
  videoRef: React.RefObject<HTMLVideoElement | null>;
}) {
  const canvasRef = React.useRef<HTMLCanvasElement>(null);

  React.useEffect(() => {
    const canvas = canvasRef.current;
    const context = canvas?.getContext("2d");
    if (!canvas || !context) return;

    const frameInterval = 1000 / 15;
    let frameId = 0;
    let lastDrawTime = 0;

    const draw = (timestamp: number) => {
      const video = videoRef.current;
      if (
        video &&
        video.readyState >= HTMLMediaElement.HAVE_CURRENT_DATA &&
        timestamp - lastDrawTime >= frameInterval
      ) {
        try {
          context.drawImage(video, 0, 0, canvas.width, canvas.height);
        } catch {
          // Frame not decodable yet — skip and retry on a later frame.
        }
        lastDrawTime = timestamp;
      }
      frameId = window.requestAnimationFrame(draw);
    };

    frameId = window.requestAnimationFrame(draw);
    return () => window.cancelAnimationFrame(frameId);
  }, [videoRef]);

  return (
    <canvas
      aria-hidden="true"
      className="pointer-events-none absolute left-1/2 top-1/2 z-0 h-[95%] w-[95%] -translate-x-1/2 -translate-y-1/2 scale-125"
      data-testid="video-review-glow"
      height={144}
      ref={canvasRef}
      style={{
        filter: "blur(80px) saturate(1.25)",
        // Radial falloff so the blurred frame never shows a hard rectangular
        // edge when the glow extends past the letterboxed video.
        maskImage: "radial-gradient(closest-side, black 65%, transparent 100%)",
        WebkitMaskImage:
          "radial-gradient(closest-side, black 65%, transparent 100%)",
      }}
      tabIndex={-1}
      width={256}
    />
  );
}

function VideoScrubber({
  ariaLabel,
  className,
  currentTime,
  duration,
  markers,
  onSeek,
  showHoverPreview = false,
  testIdPrefix,
}: {
  ariaLabel: string;
  className?: string;
  currentTime: number;
  duration: number;
  markers?: React.ReactNode;
  onSeek: (seconds: number) => void;
  showHoverPreview?: boolean;
  testIdPrefix: string;
}) {
  const sliderRef = React.useRef<HTMLDivElement>(null);
  const draggingRef = React.useRef(false);
  const [hoverRatio, setHoverRatio] = React.useState<number | null>(null);
  const progress = duration > 0 ? Math.min(currentTime / duration, 1) : 0;
  const progressPercent = progress * 100;

  const ratioFromClientX = React.useCallback(
    (target: HTMLElement, clientX: number) => {
      const rect = target.getBoundingClientRect();
      if (rect.width <= 0) return null;
      return Math.min(Math.max((clientX - rect.left) / rect.width, 0), 1);
    },
    [],
  );

  const seekFromClientX = React.useCallback(
    (target: HTMLElement, clientX: number) => {
      if (duration <= 0) return;
      const ratio = ratioFromClientX(target, clientX);
      if (ratio === null) return;
      onSeek(Math.round(ratio * duration * 1000) / 1000);
    },
    [duration, onSeek, ratioFromClientX],
  );

  const handlePointerDown = React.useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      if (duration <= 0) return;
      if (
        event.target instanceof Element &&
        event.target.closest("[data-video-review-marker]")
      ) {
        return;
      }

      event.preventDefault();
      event.stopPropagation();
      draggingRef.current = true;
      event.currentTarget.setPointerCapture(event.pointerId);
      sliderRef.current?.focus({ preventScroll: true });
      seekFromClientX(event.currentTarget, event.clientX);
    },
    [duration, seekFromClientX],
  );

  const handlePointerMove = React.useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      if (showHoverPreview && event.pointerType === "mouse") {
        setHoverRatio(ratioFromClientX(event.currentTarget, event.clientX));
      }
      if (!draggingRef.current) return;
      seekFromClientX(event.currentTarget, event.clientX);
    },
    [ratioFromClientX, seekFromClientX, showHoverPreview],
  );

  const handlePointerEnd = React.useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      if (!draggingRef.current) return;
      draggingRef.current = false;
      seekFromClientX(event.currentTarget, event.clientX);
    },
    [seekFromClientX],
  );

  const handlePointerLeave = React.useCallback(() => {
    setHoverRatio(null);
  }, []);

  const handleKeyDown = React.useCallback(
    (event: React.KeyboardEvent<HTMLDivElement>) => {
      if (duration <= 0) return;

      const smallStep = event.shiftKey ? 10 : 1;
      const largeStep = Math.max(5, duration / 10);
      let nextSeconds: number | null = null;

      switch (event.key) {
        case "ArrowLeft":
        case "ArrowDown":
          nextSeconds = currentTime - smallStep;
          break;
        case "ArrowRight":
        case "ArrowUp":
          nextSeconds = currentTime + smallStep;
          break;
        case "PageDown":
          nextSeconds = currentTime - largeStep;
          break;
        case "PageUp":
          nextSeconds = currentTime + largeStep;
          break;
        case "Home":
          nextSeconds = 0;
          break;
        case "End":
          nextSeconds = duration;
          break;
        default:
          break;
      }

      if (nextSeconds === null) return;
      event.preventDefault();
      onSeek(Math.min(Math.max(nextSeconds, 0), duration));
    },
    [currentTime, duration, onSeek],
  );

  return (
    <div
      className={cn("relative touch-none select-none", className)}
      onPointerCancel={handlePointerEnd}
      onPointerDown={handlePointerDown}
      onPointerLeave={handlePointerLeave}
      onPointerMove={handlePointerMove}
      onPointerUp={handlePointerEnd}
    >
      <div
        aria-label={ariaLabel}
        aria-valuemax={duration}
        aria-valuemin={0}
        aria-valuenow={duration > 0 ? Math.min(currentTime, duration) : 0}
        aria-valuetext={`${formatTimecode(currentTime)} / ${formatTimecode(duration)}`}
        className={cn(
          "absolute inset-0 z-10 rounded-md outline-hidden focus-visible:ring-2 focus-visible:ring-white/60",
          duration > 0 ? "cursor-pointer" : "cursor-default",
        )}
        data-testid={`${testIdPrefix}-timeline`}
        ref={sliderRef}
        role="slider"
        tabIndex={duration > 0 ? 0 : -1}
        onKeyDown={handleKeyDown}
      />
      <div
        className="pointer-events-none absolute left-0 right-0 top-1/2 h-1 -translate-y-1/2 overflow-hidden rounded-full bg-white/[0.16]"
        data-testid={`${testIdPrefix}-progress-track`}
      >
        <div
          className="h-full rounded-full bg-white"
          data-testid={`${testIdPrefix}-progress-fill`}
          style={{ width: `${progressPercent}%` }}
        />
      </div>
      {duration > 0 ? (
        <div
          className="pointer-events-none absolute top-1/2 z-20 h-4 w-0.5 -translate-x-1/2 -translate-y-1/2 rounded-full bg-white shadow-[0_0_0_0.5px_rgba(0,0,0,0.35)]"
          data-testid={`${testIdPrefix}-progress-thumb`}
          style={{ left: `${progressPercent}%` }}
        />
      ) : null}
      {showHoverPreview && hoverRatio !== null && duration > 0 ? (
        <div
          className="pointer-events-none absolute top-1/2 z-30 -translate-x-1/2 -translate-y-1/2"
          data-testid={`${testIdPrefix}-hover-cursor`}
          style={{ left: `${hoverRatio * 100}%` }}
        >
          <div className="h-5 w-0.5 rounded-full bg-white shadow-[0_0_0_0.5px_rgba(0,0,0,0.25)]" />
          <span className="absolute -top-6 left-1/2 -translate-x-1/2 whitespace-nowrap text-2xs font-medium tabular-nums text-white/90">
            {formatTimecode(hoverRatio * duration)} / {formatTimecode(duration)}
          </span>
        </div>
      ) : null}
      {markers}
    </div>
  );
}

function VolumeControl({
  expanded = false,
  muted,
  onToggleMute,
  onVolumeChange,
  volume,
}: {
  expanded?: boolean;
  muted: boolean;
  onToggleMute: () => void;
  onVolumeChange: (volume: number) => void;
  volume: number;
}) {
  const isSilent = muted || volume <= 0;
  const fillPercent = (muted ? 0 : volume) * 100;
  return (
    <div className="group/volume flex shrink-0 items-center">
      <button
        aria-label={isSilent ? "Unmute" : "Mute"}
        className="flex h-7 w-7 items-center justify-center rounded-md text-white transition-colors hover:bg-white/15 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-primary"
        type="button"
        onClick={(event) => {
          event.stopPropagation();
          onToggleMute();
        }}
      >
        {isSilent ? (
          <VolumeX className="pointer-events-none h-4 w-4" />
        ) : (
          <Volume2 className="pointer-events-none h-4 w-4" />
        )}
      </button>
      <input
        aria-label="Volume"
        className={cn(
          "video-volume-slider h-3 cursor-pointer transition-all duration-200",
          expanded
            ? "ml-1 w-16"
            : "pointer-events-none w-0 opacity-0 focus-visible:pointer-events-auto focus-visible:ml-1 focus-visible:w-16 focus-visible:opacity-100 group-hover/volume:pointer-events-auto group-hover/volume:ml-1 group-hover/volume:w-16 group-hover/volume:opacity-100",
        )}
        max={1}
        min={0}
        step={0.05}
        style={
          {
            "--video-volume-fill": `${fillPercent}%`,
          } as React.CSSProperties
        }
        type="range"
        value={muted ? 0 : volume}
        onChange={(event) => onVolumeChange(Number(event.target.value))}
        onClick={(event) => event.stopPropagation()}
        onPointerDown={(event) => event.stopPropagation()}
      />
    </div>
  );
}

function PlaybackSpeedControl({
  playbackSpeed,
  onPlaybackSpeedChange,
  size = "inline",
  testId,
}: {
  playbackSpeed: number;
  onPlaybackSpeedChange: (speed: number) => void;
  size?: "inline" | "review";
  testId: string;
}) {
  const [open, setOpen] = React.useState(false);
  const label = formatPlaybackSpeed(playbackSpeed);
  const triggerSizeClass =
    size === "review"
      ? "h-8 min-w-11 rounded-lg px-2 text-xs"
      : "h-7 min-w-10 rounded-md px-1.5 text-2xs";

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          aria-label={`Playback speed: ${label}`}
          className={cn(
            "flex shrink-0 items-center justify-center font-semibold tabular-nums text-white transition-colors hover:bg-white/15 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-white/60",
            triggerSizeClass,
            open && "bg-white/15",
          )}
          data-testid={testId}
          type="button"
          onClick={(event) => event.stopPropagation()}
        >
          {label}
        </button>
      </PopoverTrigger>
      <PopoverContent
        align="end"
        className="w-28 border-white/10 bg-black/85 p-1 text-white backdrop-blur-xl"
        data-testid={`${testId}-menu`}
        side="top"
        sideOffset={8}
        onClick={(event) => event.stopPropagation()}
      >
        <div className="px-2 pb-1 pt-1 text-2xs font-medium text-white/55">
          Speed
        </div>
        <div className="grid gap-0.5">
          {PLAYBACK_SPEEDS.map((speed) => {
            const speedLabel = formatPlaybackSpeed(speed);
            const selected = speed === playbackSpeed;
            return (
              <button
                aria-pressed={selected}
                className={cn(
                  "flex h-8 w-full items-center justify-between rounded-lg px-2 text-left text-xs font-medium tabular-nums text-white transition-colors hover:bg-white/15 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-white/60",
                  selected && "bg-white/15",
                )}
                key={speed}
                type="button"
                onClick={(event) => {
                  event.stopPropagation();
                  onPlaybackSpeedChange(speed);
                  setOpen(false);
                }}
              >
                <span>{speedLabel}</span>
                {selected ? <Check className="h-3.5 w-3.5" /> : null}
              </button>
            );
          })}
        </div>
      </PopoverContent>
    </Popover>
  );
}

export function VideoPlayer({
  src,
  poster,
  aspectRatio,
  durationSeconds,
  reviewKey,
  reviewContext,
  downloadUrl,
  filename,
}: VideoPlayerProps) {
  const persistedReviewKey = reviewKey ?? src;
  const videoRef = React.useRef<HTMLVideoElement>(null);
  const inlineSurfaceRef = React.useRef<HTMLDivElement | null>(null);
  const reviewVideoRef = React.useRef<HTMLVideoElement>(null);
  useSmoothCorners(inlineSurfaceRef);
  const [started, setStarted] = React.useState(false);
  const [isPlaying, setIsPlaying] = React.useState(false);
  const [isBuffering, setIsBuffering] = React.useState(false);
  const [hasError, setHasError] = React.useState(false);
  const [currentTime, setCurrentTimeState] = React.useState(
    () => getInlinePlaybackPosition(persistedReviewKey) ?? 0,
  );
  const currentTimeRef = React.useRef(currentTime);
  const [duration, setDuration] = React.useState(durationSeconds ?? 0);
  const [volume, setVolume] = React.useState(1);
  const [muted, setMuted] = React.useState(false);
  const [playbackSpeed, setPlaybackSpeed] = React.useState(
    DEFAULT_PLAYBACK_SPEED,
  );
  // Cache-seeded so a row evicted by the virtualized timeline remounts at the
  // ratio learned on first metadata load, not the 16/9 fallback.
  const [naturalAspectRatio, learnNaturalAspectRatio] =
    useNaturalVideoAspectRatio(src);
  const [reviewOpen, setReviewOpenState] = React.useState(() =>
    isVideoReviewOpen(persistedReviewKey),
  );
  const [reviewCurrentTime, setReviewCurrentTimeState] = React.useState(
    () => getReviewPlaybackPosition(persistedReviewKey) ?? 0,
  );
  const [pendingSeekSeconds, setPendingSeekSeconds] = React.useState<
    number | null
  >(null);
  const [inlineRenderedWidth, setInlineRenderedWidth] = React.useState<
    number | null
  >(null);

  const { onContextMenu: onSurfaceContextMenu, menu: videoContextMenu } =
    useVideoContextMenu(src, downloadUrl, filename);

  React.useEffect(() => {
    // The imeta duration seeds the timeline before metadata loads; metadata
    // (set via onLoadedMetadata below) stays authoritative once known.
    if ((durationSeconds ?? 0) > 0) {
      setDuration((current) =>
        current > 0 ? current : (durationSeconds ?? 0),
      );
    }
  }, [durationSeconds]);

  React.useEffect(() => {
    const savedCurrentTime = getInlinePlaybackPosition(persistedReviewKey) ?? 0;
    currentTimeRef.current = savedCurrentTime;
    setStarted(false);
    setCurrentTimeState(savedCurrentTime);
    setIsPlaying(false);
    setIsBuffering(false);
    setHasError(false);
    setPlaybackSpeed(DEFAULT_PLAYBACK_SPEED);
    setReviewOpenState(isVideoReviewOpen(persistedReviewKey));
    setReviewCurrentTimeState(
      getReviewPlaybackPosition(persistedReviewKey) ?? 0,
    );
  }, [persistedReviewKey]);

  React.useEffect(() => {
    return () => {
      const video = videoRef.current;
      if (!video || !Number.isFinite(video.currentTime)) {
        return;
      }
      saveInlinePlaybackPosition(
        persistedReviewKey,
        Math.max(video.currentTime, currentTimeRef.current),
        { ignoreResetToZero: true },
      );
    };
  }, [persistedReviewKey]);

  const setCurrentTime = React.useCallback(
    (seconds: number) => {
      const nextSeconds = Number.isFinite(seconds) ? Math.max(0, seconds) : 0;
      currentTimeRef.current = nextSeconds;
      saveInlinePlaybackPosition(persistedReviewKey, nextSeconds);
      setCurrentTimeState(nextSeconds);
    },
    [persistedReviewKey],
  );

  const setReviewCurrentTime = React.useCallback(
    (seconds: number) => {
      const nextSeconds = saveReviewPlaybackPosition(
        persistedReviewKey,
        seconds,
      );
      setReviewCurrentTimeState(nextSeconds);
    },
    [persistedReviewKey],
  );

  const setReviewOpen = React.useCallback(
    (open: boolean) => {
      setVideoReviewOpen(persistedReviewKey, open);
      setReviewOpenState(open);
    },
    [persistedReviewKey],
  );

  const applyPlaybackSpeed = React.useCallback(
    (video: HTMLVideoElement | null) => {
      if (!video) return;
      video.playbackRate = playbackSpeed;
    },
    [playbackSpeed],
  );

  React.useEffect(() => {
    applyPlaybackSpeed(videoRef.current);
    applyPlaybackSpeed(reviewVideoRef.current);
  }, [applyPlaybackSpeed]);

  React.useLayoutEffect(() => {
    const element = inlineSurfaceRef.current;
    if (!element) {
      setInlineRenderedWidth(null);
      return;
    }

    const updateWidth = () => {
      const width = element.getBoundingClientRect().width;
      setInlineRenderedWidth((currentWidth) =>
        currentWidth !== null && Math.abs(currentWidth - width) < 0.5
          ? currentWidth
          : width,
      );
    };

    updateWidth();

    if (typeof ResizeObserver === "undefined") {
      window.addEventListener("resize", updateWidth);
      return () => window.removeEventListener("resize", updateWidth);
    }

    const observer = new ResizeObserver(updateWidth);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  const handlePlaybackSpeedChange = React.useCallback((speed: number) => {
    if (isPlaybackSpeedOption(speed)) {
      setPlaybackSpeed(speed);
    }
  }, []);

  useSmoothPlaybackTime(videoRef, isPlaying && !reviewOpen, setCurrentTime);
  const inlineSeek = useThrottledVideoSeek(videoRef);

  const handleMediaDuration = React.useCallback((nextDuration: number) => {
    if (Number.isFinite(nextDuration) && nextDuration > 0) {
      setDuration(nextDuration);
    }
  }, []);

  const handleTogglePlay = React.useCallback(
    (event?: React.SyntheticEvent) => {
      event?.stopPropagation();
      const video = videoRef.current;
      if (!video) return;

      if (video.paused || video.ended) {
        setStarted(true);
        if (hasError) {
          setHasError(false);
          video.load();
        }
        void video.play().catch((error) => {
          if (error instanceof DOMException && error.name === "AbortError") {
            return;
          }
          setHasError(true);
          setIsPlaying(false);
          setIsBuffering(false);
        });
      } else {
        video.pause();
      }
    },
    [hasError],
  );

  const handleInlineSeek = React.useCallback(
    (seconds: number) => {
      const bounded = Math.min(
        Math.max(Number.isFinite(seconds) ? seconds : 0, 0),
        duration > 0 ? duration : Number.POSITIVE_INFINITY,
      );
      inlineSeek.requestSeek(bounded);
      setCurrentTime(bounded);
    },
    [duration, inlineSeek, setCurrentTime],
  );

  const handleToggleMute = React.useCallback(() => {
    const video = videoRef.current;
    if (!video) return;
    video.muted = !video.muted;
  }, []);

  const handleVolumeChange = React.useCallback((value: number) => {
    const video = videoRef.current;
    if (!video) return;
    video.volume = Math.min(Math.max(value, 0), 1);
    video.muted = value <= 0;
  }, []);

  const handleOpenReview = React.useCallback(
    (event?: React.SyntheticEvent) => {
      event?.stopPropagation();
      const video = videoRef.current;
      video?.pause();
      const startTime =
        video && Number.isFinite(video.currentTime)
          ? video.currentTime
          : currentTime;
      setPendingSeekSeconds(startTime);
      setReviewCurrentTime(startTime);
      setReviewOpen(true);
    },
    [currentTime, setReviewCurrentTime, setReviewOpen],
  );

  const handleReviewOpenChange = React.useCallback(
    (open: boolean) => {
      if (!open) {
        // Hand the review position back to the inline player so playback
        // resumes where the review left off.
        const video = videoRef.current;
        const reviewSeconds = getReviewPlaybackPosition(persistedReviewKey);
        if (
          video &&
          reviewSeconds !== undefined &&
          Number.isFinite(reviewSeconds)
        ) {
          video.currentTime = reviewSeconds;
          setCurrentTime(reviewSeconds);
          if (reviewSeconds > 0) {
            setStarted(true);
          }
        }
      }
      setReviewOpen(open);
    },
    [persistedReviewKey, setCurrentTime, setReviewOpen],
  );

  const handlePendingSeekConsumed = React.useCallback(() => {
    setPendingSeekSeconds(null);
  }, []);

  const timecodedComments = React.useMemo(
    () =>
      (reviewContext?.comments ?? [])
        .map(parseTimecodedComment)
        .sort(sortTimecodedComments),
    [reviewContext?.comments],
  );
  const inlineAspectRatio = aspectRatio ?? naturalAspectRatio ?? 16 / 9;
  const inlineSurfaceWidth = getInlineSurfaceWidth(inlineAspectRatio);
  const showInlineSpeedControl =
    inlineRenderedWidth !== null &&
    inlineRenderedWidth >= INLINE_SPEED_CONTROL_MIN_WIDTH;
  const inlineSurfaceStyle: React.CSSProperties = {
    aspectRatio: String(inlineAspectRatio),
    maxHeight: 256,
    width: inlineSurfaceWidth,
  };
  const showControls = started && !hasError;

  return (
    <>
      <div
        className="relative mt-3 inline-block max-w-full align-top"
        data-testid="video-player"
      >
        <div
          ref={inlineSurfaceRef}
          className="group/video relative isolate max-w-full overflow-hidden rounded-2xl border border-border/70 bg-black"
          style={inlineSurfaceStyle}
          onContextMenuCapture={onSurfaceContextMenu}
        >
          {/* Cover, not contain: when the surface's max-height clamp breaks
              the aspect match (tall videos), fill the tile and crop instead
              of letterboxing — the review overlay still shows the full
              frame. */}
          {/* biome-ignore lint/a11y/useMediaCaption: user-uploaded video, no captions available */}
          <video
            ref={videoRef}
            className="h-full w-full object-cover"
            playsInline
            poster={poster}
            preload="metadata"
            src={src}
            onClick={showControls ? handleTogglePlay : undefined}
            onDurationChange={(event) =>
              handleMediaDuration(event.currentTarget.duration)
            }
            onEnded={() => setIsPlaying(false)}
            onError={() => {
              if (started) {
                setHasError(true);
                setIsPlaying(false);
                setIsBuffering(false);
              }
            }}
            onLoadedMetadata={(event) => {
              const {
                duration: mediaDuration,
                videoHeight,
                videoWidth,
              } = event.currentTarget;
              applyPlaybackSpeed(event.currentTarget);
              handleMediaDuration(mediaDuration);
              const savedSeconds =
                getInlinePlaybackPosition(persistedReviewKey);
              if (
                savedSeconds !== undefined &&
                savedSeconds > 0 &&
                Number.isFinite(savedSeconds)
              ) {
                const restoredSeconds = Math.min(
                  savedSeconds,
                  Number.isFinite(mediaDuration) && mediaDuration > 0
                    ? mediaDuration
                    : savedSeconds,
                );
                event.currentTarget.currentTime = restoredSeconds;
                setCurrentTime(restoredSeconds);
              }
              learnNaturalAspectRatio(videoWidth, videoHeight);
            }}
            onPause={() => {
              setIsPlaying(false);
              setIsBuffering(false);
            }}
            onPlay={() => {
              setStarted(true);
              setIsPlaying(true);
            }}
            onPlaying={() => setIsBuffering(false)}
            onSeeked={inlineSeek.handleSeeked}
            onTimeUpdate={(event) =>
              setCurrentTime(event.currentTarget.currentTime)
            }
            onVolumeChange={(event) => {
              setVolume(event.currentTarget.volume);
              setMuted(event.currentTarget.muted);
            }}
            onWaiting={() => setIsBuffering(true)}
          />
          {!started && !hasError ? (
            <button
              type="button"
              aria-label="Play video"
              className="group absolute inset-0 flex cursor-pointer items-center justify-center"
              onClick={handleTogglePlay}
            >
              <span className="relative isolate flex h-14 w-14 items-center justify-center rounded-full transition-transform duration-200 ease-out group-hover:scale-105">
                <GlassSurface className="rounded-full" />
                <Play className="h-6 w-6 fill-white text-white" />
              </span>
            </button>
          ) : null}
          {isBuffering && !hasError ? (
            <div className="pointer-events-none absolute inset-0 flex items-center justify-center">
              <span className="relative isolate flex h-14 w-14 items-center justify-center rounded-full">
                <GlassSurface className="rounded-full" />
                <Spinner className="text-white" size={28} />
              </span>
            </div>
          ) : null}
          {hasError ? (
            <button
              type="button"
              aria-label="Retry loading video"
              className="group absolute inset-0 flex cursor-pointer flex-col items-center justify-center gap-2"
              onClick={handleTogglePlay}
            >
              <span className="relative isolate flex h-14 w-14 items-center justify-center rounded-full transition-transform duration-200 ease-out group-hover:scale-105">
                <GlassSurface className="rounded-full" />
                <AlertCircle className="h-6 w-6 text-white" />
              </span>
              <span className="rounded-md bg-black/50 px-2 py-1 text-xs text-white backdrop-blur-sm">
                Failed to load — tap to retry
              </span>
            </button>
          ) : null}
          {/* Slide (not fade) the pill out: animating opacity on an ancestor
              of a backdrop-filter flattens the glass into a plain fill
              mid-transition, which reads as a flicker. The video container's
              overflow-hidden clips the slid-out pill. */}
          {showControls ? (
            <div
              className={cn(
                "absolute inset-x-1.5 bottom-1.5 z-10 transition-transform duration-300 ease-out",
                isPlaying &&
                  "pointer-events-none translate-y-[150%] focus-within:pointer-events-auto focus-within:translate-y-0 group-hover/video:pointer-events-auto group-hover/video:translate-y-0",
              )}
              data-testid="video-inline-controls"
            >
              <div className="relative isolate flex items-center gap-1 rounded-[10px] px-1.5 py-1">
                <GlassSurface />
                <button
                  aria-label={isPlaying ? "Pause video" : "Play video"}
                  className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-white transition-colors hover:bg-white/15 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-primary"
                  type="button"
                  onClick={handleTogglePlay}
                >
                  {isPlaying ? (
                    <Pause className="pointer-events-none h-4 w-4 fill-white" />
                  ) : (
                    <Play className="pointer-events-none h-4 w-4 fill-white" />
                  )}
                </button>
                <span
                  className="shrink-0 text-2xs font-medium tabular-nums leading-none text-white"
                  data-testid="video-inline-time"
                >
                  {formatTimecode(currentTime)}
                </span>
                <VideoScrubber
                  ariaLabel="Video progress"
                  className="group h-7 min-w-0 flex-1"
                  currentTime={currentTime}
                  duration={duration}
                  onSeek={handleInlineSeek}
                  testIdPrefix="video-inline"
                />
                <span
                  className="shrink-0 text-2xs font-medium tabular-nums leading-none text-white/70"
                  data-testid="video-inline-duration"
                >
                  {formatTimecode(duration)}
                </span>
                {showInlineSpeedControl ? (
                  <PlaybackSpeedControl
                    playbackSpeed={playbackSpeed}
                    testId="video-inline-speed"
                    onPlaybackSpeedChange={handlePlaybackSpeedChange}
                  />
                ) : null}
                <VolumeControl
                  muted={muted}
                  volume={volume}
                  onToggleMute={handleToggleMute}
                  onVolumeChange={handleVolumeChange}
                />
              </div>
            </div>
          ) : null}
        </div>
        {!hasError ? (
          <button
            type="button"
            aria-label="Open video review"
            data-video-review-launcher=""
            title="Open video review"
            className="group absolute right-2 top-2 z-10 flex h-8 w-8 items-center justify-center rounded-full text-white shadow-sm outline-hidden focus-visible:ring-2 focus-visible:ring-white/60"
            onClick={handleOpenReview}
          >
            <span className="relative isolate flex h-full w-full items-center justify-center rounded-full transition-transform duration-200 ease-out group-hover:scale-110">
              <GlassSurface className="rounded-full" />
              <Maximize2 className="pointer-events-none h-4 w-4" />
            </span>
          </button>
        ) : null}
      </div>
      {videoContextMenu}
      <VideoReviewDialog
        comments={timecodedComments}
        currentTime={reviewCurrentTime}
        duration={duration}
        onCurrentTimeChange={setReviewCurrentTime}
        onDurationChange={handleMediaDuration}
        onOpenChange={handleReviewOpenChange}
        onPendingSeekConsumed={handlePendingSeekConsumed}
        onPlaybackSpeedChange={handlePlaybackSpeedChange}
        open={reviewOpen}
        pendingSeekSeconds={pendingSeekSeconds}
        playbackSpeed={playbackSpeed}
        poster={poster}
        reviewContext={reviewContext}
        src={src}
        title={reviewContext?.title ?? fileNameFromUrl(src)}
        videoRef={reviewVideoRef}
      />
    </>
  );
}

function VideoReviewDialog({
  comments,
  currentTime,
  duration,
  onCurrentTimeChange,
  onDurationChange,
  onOpenChange,
  onPendingSeekConsumed,
  onPlaybackSpeedChange,
  open,
  pendingSeekSeconds,
  playbackSpeed,
  poster,
  reviewContext,
  src,
  title,
  videoRef,
}: {
  comments: TimecodedComment[];
  currentTime: number;
  duration: number;
  onCurrentTimeChange: (seconds: number) => void;
  onDurationChange: (seconds: number) => void;
  onOpenChange: (open: boolean) => void;
  onPendingSeekConsumed: () => void;
  onPlaybackSpeedChange: (speed: number) => void;
  open: boolean;
  pendingSeekSeconds: number | null;
  playbackSpeed: number;
  poster?: string;
  reviewContext?: VideoReviewContext;
  src: string;
  title: string;
  videoRef: React.RefObject<HTMLVideoElement | null>;
}) {
  const [isPlaying, setIsPlaying] = React.useState(false);
  const [isPosting, setIsPosting] = React.useState(false);
  const [errorMessage, setErrorMessage] = React.useState<string | null>(null);
  const [isEmojiPickerOpen, setIsEmojiPickerOpen] = React.useState(false);
  const [isPanelOpen, setIsPanelOpen] = React.useState(true);
  const [postAtCurrentFrame, setPostAtCurrentFrame] = React.useState(true);
  const [isComposerMounted, setIsComposerMounted] = React.useState(false);
  const [replyTarget, setReplyTarget] = React.useState<TimecodedComment | null>(
    null,
  );
  const [volume, setVolume] = React.useState(1);
  const [muted, setMuted] = React.useState(false);
  const [mediaRatio, setMediaRatio] = React.useState<number | null>(null);
  const [videoAreaSize, setVideoAreaSize] = React.useState<{
    height: number;
    width: number;
  } | null>(null);
  const videoAreaRef = React.useRef<HTMLDivElement | null>(null);
  const [optimisticComments, setOptimisticComments] = React.useState<
    VideoReviewComment[]
  >([]);
  const currentTimeRef = React.useRef(currentTime);
  const replyTargetRef = React.useRef(replyTarget);
  const postAtCurrentFrameRef = React.useRef(postAtCurrentFrame);
  const composerWrapperRef = React.useRef<HTMLDivElement | null>(null);
  currentTimeRef.current = currentTime;
  replyTargetRef.current = replyTarget;
  postAtCurrentFrameRef.current = postAtCurrentFrame;

  const canComment =
    Boolean(reviewContext?.onSendComment) && !reviewContext?.disabled;
  const isPostingReviewItem = Boolean(reviewContext?.isSending) || isPosting;
  const canPost = canComment && !isPostingReviewItem;
  const postDisabledReason = !reviewContext?.onSendComment
    ? "Commenting is unavailable for this video."
    : reviewContext.disabled
      ? "Wait for the video message to finish sending."
      : isPostingReviewItem
        ? "Posting..."
        : null;
  const visibleDuration = duration > 0 ? duration : 0;
  const displayComments = React.useMemo(() => {
    const confirmedBodies = new Set(
      comments.map((comment) => comment.comment.body),
    );
    const pendingComments = optimisticComments
      .filter((comment) => !confirmedBodies.has(comment.body))
      .map(parseTimecodedComment);
    return [...comments, ...pendingComments].sort(sortTimecodedComments);
  }, [comments, optimisticComments]);
  const displayMarkerComments = React.useMemo(
    () =>
      displayComments.filter(
        (comment) =>
          comment.seconds !== null &&
          visibleDuration > 0 &&
          comment.seconds <= visibleDuration,
      ),
    [displayComments, visibleDuration],
  );
  // Thread the flat comment list: replies (comments whose parent is another
  // comment) nest one level under their top-level ancestor, frame.io-style.
  const commentThreads = React.useMemo(() => {
    const byId = new Map(
      displayComments.map((item) => [item.comment.id, item]),
    );
    const topLevel = displayComments.filter(
      (item) => !item.comment.parentId || !byId.has(item.comment.parentId),
    );
    const repliesByTopId = new Map<string, TimecodedComment[]>();
    for (const item of displayComments) {
      if (!item.comment.parentId || !byId.has(item.comment.parentId)) continue;
      // Walk up to the top-level ancestor (deep reply chains flatten).
      let ancestor = item;
      const visited = new Set<string>([item.comment.id]);
      while (
        ancestor.comment.parentId &&
        byId.has(ancestor.comment.parentId) &&
        !visited.has(ancestor.comment.parentId)
      ) {
        visited.add(ancestor.comment.parentId);
        ancestor = byId.get(ancestor.comment.parentId) as TimecodedComment;
      }
      const replies = repliesByTopId.get(ancestor.comment.id) ?? [];
      replies.push(item);
      repliesByTopId.set(ancestor.comment.id, replies);
    }
    for (const replies of repliesByTopId.values()) {
      replies.sort((a, b) => a.comment.createdAt - b.comment.createdAt);
    }
    return { repliesByTopId, topLevel };
  }, [displayComments]);
  // A read-only panel (no send capability) is still useful when comments
  // exist; with neither, drop the column and let the video own the dialog.
  const showCommentsPanel =
    Boolean(reviewContext?.onSendComment) || displayComments.length > 0;

  const syncCurrentTime = React.useCallback(
    (seconds: number) => {
      const nextSeconds = Number.isFinite(seconds) ? Math.max(0, seconds) : 0;
      currentTimeRef.current = nextSeconds;
      onCurrentTimeChange(nextSeconds);
    },
    [onCurrentTimeChange],
  );

  const boundSeconds = React.useCallback(
    (seconds: number) => {
      const nextSeconds = Number.isFinite(seconds) ? Math.max(0, seconds) : 0;
      return duration > 0 ? Math.min(nextSeconds, duration) : nextSeconds;
    },
    [duration],
  );

  React.useEffect(() => {
    if (!open) {
      setIsComposerMounted(false);
      return;
    }
    // Two frames: one for the dialog to paint, one for the browser to
    // composite, then mount the heavyweight Tiptap composer.
    let secondFrameId = 0;
    const firstFrameId = window.requestAnimationFrame(() => {
      secondFrameId = window.requestAnimationFrame(() =>
        setIsComposerMounted(true),
      );
    });
    return () => {
      window.cancelAnimationFrame(firstFrameId);
      window.cancelAnimationFrame(secondFrameId);
    };
  }, [open]);

  React.useEffect(() => {
    if (!open) return;
    const element = videoAreaRef.current;
    if (!element) return;

    const update = () => {
      const rect = element.getBoundingClientRect();
      setVideoAreaSize({ height: rect.height, width: rect.width });
    };
    update();
    const observer = new ResizeObserver(update);
    observer.observe(element);
    return () => observer.disconnect();
  }, [open]);

  // Size the video wrapper to the frame itself (not the letterboxed area) so
  // the corner radius, glow, and floating controls all hug the picture.
  const fittedVideoStyle = React.useMemo<React.CSSProperties>(() => {
    if (
      !mediaRatio ||
      !videoAreaSize ||
      videoAreaSize.width <= 0 ||
      videoAreaSize.height <= 0
    ) {
      return { height: "100%", width: "100%" };
    }
    const width = Math.min(
      videoAreaSize.width,
      videoAreaSize.height * mediaRatio,
    );
    return { aspectRatio: String(mediaRatio), width };
  }, [mediaRatio, videoAreaSize]);

  const reviewSeek = useThrottledVideoSeek(videoRef);

  const setVideoTime = React.useCallback(
    (seconds: number) => {
      const boundedSeconds = boundSeconds(seconds);
      reviewSeek.requestSeek(boundedSeconds);
      syncCurrentTime(boundedSeconds);
    },
    [boundSeconds, reviewSeek, syncCurrentTime],
  );

  React.useEffect(() => {
    if (!open) return;
    if (pendingSeekSeconds === null) return;
    const video = videoRef.current;
    if (!video) return;
    video.currentTime = pendingSeekSeconds;
    syncCurrentTime(pendingSeekSeconds);
    onPendingSeekConsumed();
  }, [
    onPendingSeekConsumed,
    open,
    pendingSeekSeconds,
    syncCurrentTime,
    videoRef,
  ]);

  useSmoothPlaybackTime(videoRef, open && isPlaying, syncCurrentTime);

  const togglePlay = React.useCallback(() => {
    const video = videoRef.current;
    if (!video) return;

    if (video.paused || video.ended) {
      void video.play().catch(() => undefined);
    } else {
      video.pause();
    }
  }, [videoRef]);

  const readAuthoringSeconds = React.useCallback(() => {
    const video = videoRef.current;
    return boundSeconds(video?.currentTime ?? currentTimeRef.current);
  }, [boundSeconds, videoRef]);

  const pauseForCommentAuthoring = React.useCallback(() => {
    const video = videoRef.current;
    if (video && !video.paused) {
      video.pause();
    }
  }, [videoRef]);

  const handleToggleMute = React.useCallback(() => {
    const video = videoRef.current;
    if (!video) return;
    video.muted = !video.muted;
  }, [videoRef]);

  const handleVolumeChange = React.useCallback(
    (value: number) => {
      const video = videoRef.current;
      if (!video) return;
      video.volume = Math.min(Math.max(value, 0), 1);
      video.muted = value <= 0;
    },
    [videoRef],
  );

  React.useEffect(() => {
    if (!open) return;
    const video = videoRef.current;
    if (!video) return;
    video.playbackRate = playbackSpeed;
  }, [open, playbackSpeed, videoRef]);

  const postContent = React.useCallback(
    async (
      content: string,
      options?: {
        mediaTags?: string[][];
        mentionPubkeys?: string[];
        replyTo?: VideoReviewComment | null;
        secondsOverride?: number;
        /** Default true — quick reactions always stamp; the composer toggle
         * can opt out. */
        stampTimecode?: boolean;
      },
    ) => {
      const trimmed = content.trim();
      if (!trimmed || !reviewContext?.onSendComment || !canPost) return;

      const replyTo = options?.replyTo ?? null;
      const authoredSeconds =
        typeof options?.secondsOverride === "number" &&
        Number.isFinite(options.secondsOverride)
          ? boundSeconds(options.secondsOverride)
          : readAuthoringSeconds();
      // Replies attach to a comment's existing moment, and the composer
      // toggle lets plain comments skip the frame stamp entirely.
      const stampTimecode = !replyTo && (options?.stampTimecode ?? true);
      const body = stampTimecode
        ? `[${formatCommentTimecode(authoredSeconds)}] ${trimmed}`
        : trimmed;
      const optimisticComment: VideoReviewComment = {
        id: `optimistic-video-review-${Date.now()}-${Math.random()
          .toString(36)
          .slice(2)}`,
        author: "You",
        avatarUrl: null,
        body,
        createdAt: Math.floor(Date.now() / 1000),
        parentId: replyTo?.id ?? null,
        time: "now",
      };

      setOptimisticComments((current) => [...current, optimisticComment]);
      setIsPosting(true);
      setErrorMessage(null);
      try {
        await reviewContext.onSendComment(
          body,
          options?.mentionPubkeys ?? [],
          options?.mediaTags,
          replyTo?.id,
        );
        if (replyTo) {
          setReplyTarget(null);
        }
      } catch (error) {
        setOptimisticComments((current) =>
          current.filter((comment) => comment.id !== optimisticComment.id),
        );
        setErrorMessage(
          error instanceof Error ? error.message : "Failed to post comment.",
        );
        // Rethrow so the composer restores the draft it cleared on submit.
        throw error;
      } finally {
        setIsPosting(false);
      }
    },
    [boundSeconds, canPost, readAuthoringSeconds, reviewContext],
  );

  const handleComposerSend = React.useCallback(
    async (
      content: string,
      mentionPubkeys: string[],
      mediaTags?: string[][],
    ) => {
      await postContent(content, {
        mediaTags,
        mentionPubkeys,
        replyTo: replyTargetRef.current?.comment ?? null,
        stampTimecode: postAtCurrentFrameRef.current,
      });
    },
    [postContent],
  );

  const focusComposerInput = React.useCallback(() => {
    window.requestAnimationFrame(() => {
      composerWrapperRef.current
        ?.querySelector<HTMLElement>("[data-testid='message-input']")
        ?.focus();
    });
  }, []);

  const handleReplyPress = React.useCallback(
    (item: TimecodedComment) => {
      setReplyTarget(item);
      focusComposerInput();
    },
    [focusComposerInput],
  );

  const handleCancelReply = React.useCallback(() => {
    setReplyTarget(null);
  }, []);

  const handleReactionPress = React.useCallback(
    (emoji: string) => {
      void postContent(emoji, {
        secondsOverride: readAuthoringSeconds(),
      }).catch(() => undefined);
    },
    [postContent, readAuthoringSeconds],
  );

  const handleEmojiPickerSelect = React.useCallback(
    (emoji: string) => {
      void postContent(emoji)
        .catch(() => undefined)
        .finally(() => setIsEmojiPickerOpen(false));
    },
    [postContent],
  );

  const handleClose = React.useCallback(() => {
    onOpenChange(false);
  }, [onOpenChange]);

  React.useEffect(() => {
    if (!open) return;

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      if (isEmojiPickerOpen) {
        setIsEmojiPickerOpen(false);
      } else {
        onOpenChange(false);
      }
    };

    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [isEmojiPickerOpen, onOpenChange, open]);

  if (!open) return null;

  return createPortal(
    <div
      className={cn(
        "dark video-review-theme fixed inset-0 z-50 flex min-h-0 min-w-0 items-center justify-center bg-black/75 p-4 text-foreground sm:p-8 lg:p-10",
        MODAL_BACKDROP_BLUR_CLASS,
      )}
    >
      <button
        aria-label="Close video review"
        className="absolute inset-0 cursor-default"
        data-testid="video-review-backdrop"
        type="button"
        onClick={handleClose}
      />
      <div
        aria-describedby="video-review-description"
        aria-labelledby="video-review-title"
        aria-modal="true"
        className="relative z-10 flex h-full max-h-[980px] min-h-0 w-full max-w-[1520px] min-w-0 flex-col overflow-hidden rounded-2xl border border-border bg-background text-foreground shadow-2xl outline-hidden"
        data-testid="video-review-dialog"
        role="dialog"
      >
        <h2 className="sr-only" id="video-review-title">
          Video review
        </h2>
        <p className="sr-only" id="video-review-description">
          Review video comments and timecoded replies.
        </p>

        <header className="flex h-12 shrink-0 items-center gap-3 border-b border-border bg-background px-4">
          <div className="min-w-0 flex-1">
            <p className="truncate text-sm font-semibold text-foreground">
              {title}
            </p>
          </div>
          {showCommentsPanel ? (
            <Button
              aria-label={isPanelOpen ? "Hide comments" : "Show comments"}
              aria-pressed={isPanelOpen}
              className={cn(
                "h-8 w-8 rounded-lg border border-border bg-muted/40 text-foreground hover:bg-muted",
                isPanelOpen && "bg-muted",
              )}
              data-testid="video-review-toggle-comments"
              size="icon"
              type="button"
              variant="ghost"
              onClick={() => setIsPanelOpen((open) => !open)}
            >
              <PanelRight className="h-4 w-4" />
            </Button>
          ) : null}
          <Button
            aria-label="Close video review"
            className="h-8 w-8 rounded-lg border border-border bg-muted/40 text-foreground hover:bg-muted"
            size="icon"
            type="button"
            variant="ghost"
            onClick={handleClose}
          >
            <X className="h-4 w-4" />
          </Button>
        </header>

        <div className="flex min-h-0 min-w-0 flex-1">
          <section className="flex min-h-0 min-w-0 flex-1 flex-col bg-black">
            <div
              className="relative flex min-h-0 flex-1 items-center justify-center p-4 pb-3 lg:p-6 lg:pb-4"
              ref={videoAreaRef}
            >
              <div
                className="relative isolate flex max-h-full min-w-0 max-w-full items-center justify-center"
                style={fittedVideoStyle}
              >
                <VideoGlow videoRef={videoRef} />
                <div className="relative z-10 h-full w-full overflow-hidden rounded-lg">
                  {/* biome-ignore lint/a11y/useMediaCaption: user-uploaded video, no captions available */}
                  <video
                    ref={videoRef}
                    className="h-full w-full min-h-0 object-contain"
                    playsInline
                    poster={poster}
                    preload="metadata"
                    src={src}
                    onClick={togglePlay}
                    onDurationChange={(event) =>
                      onDurationChange(event.currentTarget.duration)
                    }
                    onEnded={(event) => {
                      syncCurrentTime(event.currentTarget.currentTime);
                      setIsPlaying(false);
                    }}
                    onLoadedMetadata={(event) => {
                      event.currentTarget.playbackRate = playbackSpeed;
                      onDurationChange(event.currentTarget.duration);
                      const { videoHeight, videoWidth } = event.currentTarget;
                      if (videoWidth > 0 && videoHeight > 0) {
                        setMediaRatio(videoWidth / videoHeight);
                      }
                      if (
                        pendingSeekSeconds !== null &&
                        pendingSeekSeconds > 0
                      ) {
                        event.currentTarget.currentTime = pendingSeekSeconds;
                        syncCurrentTime(pendingSeekSeconds);
                      }
                    }}
                    onPause={(event) => {
                      syncCurrentTime(event.currentTarget.currentTime);
                      setIsPlaying(false);
                    }}
                    onPlay={(event) => {
                      syncCurrentTime(event.currentTarget.currentTime);
                      setIsPlaying(true);
                    }}
                    onSeeked={reviewSeek.handleSeeked}
                    onTimeUpdate={(event) => {
                      syncCurrentTime(event.currentTarget.currentTime);
                    }}
                    onVolumeChange={(event) => {
                      setVolume(event.currentTarget.volume);
                      setMuted(event.currentTarget.muted);
                    }}
                  />
                </div>
                <div className="absolute inset-x-2 bottom-2 z-20 sm:inset-x-4 sm:bottom-3">
                  <div className="relative isolate flex items-center gap-2 rounded-xl px-2 py-1.5">
                    <GlassSurface />
                    <button
                      aria-label={
                        isPlaying ? "Pause review video" : "Play review video"
                      }
                      className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-white transition-colors hover:bg-white/15 outline-hidden focus-visible:ring-2 focus-visible:ring-white/60"
                      type="button"
                      onClick={togglePlay}
                    >
                      {isPlaying ? (
                        <Pause className="h-4 w-4 fill-white text-white" />
                      ) : (
                        <Play className="h-4 w-4 fill-white text-white" />
                      )}
                    </button>
                    <span
                      className="shrink-0 text-xs font-medium tabular-nums text-white"
                      data-testid="video-review-time-current"
                    >
                      {formatTimecode(currentTime)}
                    </span>
                    <VideoScrubber
                      ariaLabel="Video timeline"
                      className="group h-8 min-w-0 flex-1"
                      currentTime={currentTime}
                      duration={visibleDuration}
                      onSeek={setVideoTime}
                      showHoverPreview
                      testIdPrefix="video-review"
                      markers={displayMarkerComments.map((item) => {
                        const seconds = item.seconds ?? 0;
                        const left = `${Math.min((seconds / visibleDuration) * 100, 100)}%`;
                        return (
                          <Tooltip key={item.comment.id}>
                            <TooltipTrigger asChild>
                              <button
                                aria-label={`Seek to ${item.timecode}`}
                                className="absolute top-1/2 z-20 flex h-5 w-5 -translate-x-1/2 -translate-y-1/2 items-center justify-center rounded-full border border-white/40 bg-black/60 shadow-md transition-transform hover:scale-110 outline-hidden focus-visible:ring-2 focus-visible:ring-white/60"
                                data-video-review-marker=""
                                style={{ left }}
                                type="button"
                                onClick={() => setVideoTime(seconds)}
                              >
                                <UserAvatar
                                  avatarUrl={item.comment.avatarUrl ?? null}
                                  className="h-4 w-4 shadow-none"
                                  displayName={item.comment.author}
                                  size="xs"
                                />
                              </button>
                            </TooltipTrigger>
                            <TooltipContent side="top">
                              {item.timecode} · {item.comment.author}
                            </TooltipContent>
                          </Tooltip>
                        );
                      })}
                    />
                    <span
                      className="shrink-0 text-xs font-medium tabular-nums text-white/70"
                      data-testid="video-review-time-duration"
                    >
                      {formatTimecode(visibleDuration)}
                    </span>
                    <PlaybackSpeedControl
                      playbackSpeed={playbackSpeed}
                      size="review"
                      testId="video-review-speed"
                      onPlaybackSpeedChange={onPlaybackSpeedChange}
                    />
                    <VolumeControl
                      expanded
                      muted={muted}
                      volume={volume}
                      onToggleMute={handleToggleMute}
                      onVolumeChange={handleVolumeChange}
                    />
                  </div>
                </div>
              </div>
            </div>

            {reviewContext?.onSendComment ? (
              <div className="relative z-10 flex shrink-0 items-center justify-center px-4 pb-4 pt-1">
                <div
                  className="relative isolate flex flex-wrap items-center justify-center gap-1 rounded-full px-2 py-1 text-white"
                  data-testid="video-review-reaction-tray"
                >
                  <GlassSurface />
                  {QUICK_REACTIONS.map((emoji, index) => (
                    <button
                      aria-label={`React ${emoji} at ${formatTimecode(currentTime)}`}
                      className="flex h-8 w-8 items-center justify-center rounded-full text-lg transition-colors hover:bg-white/10 disabled:cursor-not-allowed disabled:opacity-35"
                      data-testid={`video-review-reaction-${index}`}
                      disabled={!canPost}
                      key={emoji}
                      title={postDisabledReason ?? undefined}
                      type="button"
                      onClick={() => handleReactionPress(emoji)}
                    >
                      <span className="pointer-events-none">{emoji}</span>
                    </button>
                  ))}
                  <span className="mx-1 h-6 w-px bg-white/15" />
                  <div className="relative">
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <button
                          aria-label="More reactions"
                          aria-pressed={isEmojiPickerOpen}
                          className={cn(
                            "flex h-8 w-8 items-center justify-center rounded-full text-white/80 transition-colors hover:bg-white/10 disabled:cursor-not-allowed disabled:opacity-35",
                            isEmojiPickerOpen && "bg-white/15 text-white",
                          )}
                          disabled={!canPost}
                          title={postDisabledReason ?? "More reactions"}
                          type="button"
                          onClick={() => setIsEmojiPickerOpen((open) => !open)}
                        >
                          <SmilePlus className="pointer-events-none h-4 w-4" />
                        </button>
                      </TooltipTrigger>
                      <TooltipContent>More reactions</TooltipContent>
                    </Tooltip>
                    {isEmojiPickerOpen ? (
                      <div
                        className="absolute bottom-full right-0 z-30 mb-2 overflow-hidden rounded-2xl shadow-2xl"
                        data-testid="video-review-emoji-picker"
                      >
                        <EmojiPicker
                          autoFocus
                          onSelect={handleEmojiPickerSelect}
                        />
                      </div>
                    ) : null}
                  </div>
                </div>
              </div>
            ) : null}
          </section>

          {showCommentsPanel ? (
            <aside
              className="min-h-0 shrink-0 overflow-hidden transition-[width] duration-200 ease-out"
              data-testid="video-review-comments-panel"
              inert={!isPanelOpen || undefined}
              style={{ width: isPanelOpen ? 380 : 0 }}
            >
              <div className="flex h-full w-[380px] min-h-0 flex-col border-l border-border bg-background">
                <div className="flex h-12 shrink-0 items-center border-b border-border px-4">
                  <div className="flex min-w-0 items-center gap-2">
                    <MessageCircle className="h-4 w-4 text-muted-foreground" />
                    <h3 className="text-sm font-semibold text-foreground">
                      Comments
                    </h3>
                  </div>
                  <span className="ml-auto rounded-full bg-muted px-2 py-0.5 text-xs text-muted-foreground">
                    {displayComments.length}
                  </span>
                </div>

                <div
                  className="min-h-0 flex-1 overflow-y-auto px-4 py-3"
                  data-testid="video-review-comments"
                >
                  {commentThreads.topLevel.length > 0 ? (
                    <div className="space-y-3">
                      {commentThreads.topLevel.map((item) => (
                        <VideoReviewCommentCard
                          canReply={canComment}
                          item={item}
                          key={item.comment.id}
                          onReply={handleReplyPress}
                          onSeek={setVideoTime}
                          onToggleReaction={
                            reviewContext?.onToggleCommentReaction
                          }
                          replies={
                            commentThreads.repliesByTopId.get(
                              item.comment.id,
                            ) ?? []
                          }
                        />
                      ))}
                    </div>
                  ) : (
                    <div className="rounded-lg border border-dashed border-border px-4 py-6 text-center">
                      <p className="text-sm font-medium text-muted-foreground">
                        No comments yet
                      </p>
                    </div>
                  )}
                </div>

                <div className="shrink-0 border-t border-border">
                  <div className="flex items-center gap-2.5 px-4 pb-3 pt-4">
                    <span
                      className={cn(
                        "rounded-md px-2 py-1 font-mono text-xs font-semibold transition-colors",
                        !replyTarget && postAtCurrentFrame
                          ? TIMECODE_ACCENT_CLASS
                          : "bg-muted text-muted-foreground/70",
                      )}
                      data-testid="video-review-composer-timecode"
                    >
                      {formatTimecode(currentTime)}
                    </span>
                    {replyTarget ? (
                      <span className="text-2xs text-muted-foreground">
                        Replying to {replyTarget.comment.author}
                      </span>
                    ) : (
                      <label
                        className="flex cursor-pointer select-none items-center gap-1.5 text-2xs text-muted-foreground"
                        htmlFor="video-review-frame-toggle"
                      >
                        <Checkbox
                          checked={postAtCurrentFrame}
                          className="h-3.5 w-3.5"
                          data-testid="video-review-frame-toggle"
                          id="video-review-frame-toggle"
                          onCheckedChange={(checked) =>
                            setPostAtCurrentFrame(checked === true)
                          }
                        />
                        Comment at current frame
                      </label>
                    )}
                  </div>
                  {canComment && !isComposerMounted ? (
                    <div aria-hidden="true" className="px-3 pb-3.5">
                      <div className="h-[88px] rounded-2xl border border-border/50 bg-muted/20" />
                    </div>
                  ) : null}
                  {canComment && isComposerMounted ? (
                    <div
                      className="[&_footer>div[aria-hidden]]:hidden"
                      ref={composerWrapperRef}
                      onFocusCapture={pauseForCommentAuthoring}
                    >
                      <MessageComposer
                        channelId={reviewContext?.channelId ?? null}
                        channelName={
                          reviewContext?.channelName ?? "video-review"
                        }
                        channelType={reviewContext?.channelType ?? null}
                        containerClassName="px-3 pb-3.5"
                        disabled={!canComment}
                        draftKey={
                          reviewContext?.rootEventId
                            ? `video-review:${reviewContext.rootEventId}`
                            : undefined
                        }
                        isSending={isPostingReviewItem}
                        onCancelReply={
                          replyTarget ? handleCancelReply : undefined
                        }
                        onSend={handleComposerSend}
                        placeholder={
                          replyTarget ? undefined : "Leave your comment..."
                        }
                        profiles={reviewContext?.profiles}
                        replyTarget={
                          replyTarget
                            ? {
                                author: replyTarget.comment.author,
                                body:
                                  replyTarget.text || replyTarget.comment.body,
                                id: replyTarget.comment.id,
                              }
                            : null
                        }
                        typingParentEventId={reviewContext?.rootEventId ?? null}
                        typingRootEventId={reviewContext?.rootEventId ?? null}
                      />
                    </div>
                  ) : (
                    <p className="px-4 pb-4 pt-2 text-xs text-muted-foreground">
                      {postDisabledReason}
                    </p>
                  )}
                  {errorMessage ? (
                    <p className="px-4 pb-3 text-xs text-destructive">
                      {errorMessage}
                    </p>
                  ) : null}
                </div>
              </div>
            </aside>
          ) : null}
        </div>
      </div>
    </div>,
    document.body,
  );
}

function VideoReviewCommentCard({
  item,
  canReply,
  onReply,
  onSeek,
  onToggleReaction,
  replies,
}: {
  canReply: boolean;
  item: TimecodedComment;
  onReply: (item: TimecodedComment) => void;
  onSeek: (seconds: number) => void;
  onToggleReaction?: (
    comment: VideoReviewComment,
    emoji: string,
    remove: boolean,
  ) => Promise<void>;
  replies: TimecodedComment[];
}) {
  return (
    <article className="rounded-lg bg-muted/40 p-3 text-sm text-foreground">
      <VideoReviewCommentBody
        canReply={canReply}
        item={item}
        onReply={onReply}
        onSeek={onSeek}
        onToggleReaction={onToggleReaction}
      />
      {replies.length > 0 ? (
        <div className="mt-3 space-y-3 border-l border-border pl-3">
          {replies.map((reply) => (
            <VideoReviewCommentBody
              canReply={canReply}
              item={reply}
              key={reply.comment.id}
              onReply={onReply}
              onSeek={onSeek}
              onToggleReaction={onToggleReaction}
            />
          ))}
        </div>
      ) : null}
    </article>
  );
}

function VideoReviewCommentBody({
  canReply,
  item,
  onReply,
  onSeek,
  onToggleReaction,
}: {
  canReply: boolean;
  item: TimecodedComment;
  onReply: (item: TimecodedComment) => void;
  onSeek: (seconds: number) => void;
  onToggleReaction?: (
    comment: VideoReviewComment,
    emoji: string,
    remove: boolean,
  ) => Promise<void>;
}) {
  const reactions = item.comment.reactions ?? [];
  const text = item.text || item.comment.body;
  const timecodeButton =
    item.seconds !== null && item.timecode ? (
      <button
        aria-label={`Jump to ${item.timecode}`}
        className={cn(
          "inline-flex h-5 shrink-0 items-center rounded px-1.5 align-middle font-mono text-2xs font-semibold outline-hidden transition-colors focus-visible:ring-2 focus-visible:ring-white/60",
          TIMECODE_ACCENT_CLASS,
          TIMECODE_ACCENT_HOVER_CLASS,
        )}
        data-testid="video-review-comment-timecode"
        type="button"
        onClick={() => onSeek(item.seconds ?? 0)}
      >
        {item.timecode}
      </button>
    ) : null;

  return (
    <div>
      <div className="flex items-center gap-2">
        <UserAvatar
          avatarUrl={item.comment.avatarUrl ?? null}
          className="h-6 w-6 shadow-none"
          displayName={item.comment.author}
          size="xs"
        />
        <p className="truncate text-sm font-semibold text-foreground">
          {item.comment.author}
        </p>
        <p className="shrink-0 text-xs text-muted-foreground">
          {item.comment.time}
        </p>
        {reactions.some((reaction) => reaction.reactedByCurrentUser) ? (
          <Check
            className="ml-auto h-4 w-4 shrink-0 text-primary"
            aria-hidden
          />
        ) : null}
      </div>
      <p className="mt-1.5 leading-5">
        {timecodeButton ? <>{timecodeButton} </> : null}
        {text}
      </p>
      {reactions.length > 0 ? (
        <div className="mt-2 flex flex-wrap gap-1.5">
          {reactions.map((reaction) => (
            <button
              aria-label={`Toggle ${reaction.emoji} reaction`}
              aria-pressed={reaction.reactedByCurrentUser}
              className={cn(
                "flex h-7 items-center gap-1 rounded-full border px-2 text-xs transition-colors",
                reaction.reactedByCurrentUser
                  ? "border-primary/60 bg-primary/20 text-primary"
                  : "border-border bg-muted/40 text-muted-foreground hover:bg-muted",
              )}
              disabled={!onToggleReaction}
              key={reaction.emoji}
              type="button"
              onClick={() => {
                void onToggleReaction?.(
                  item.comment,
                  reaction.emoji,
                  Boolean(reaction.reactedByCurrentUser),
                );
              }}
            >
              {reaction.emojiUrl ? (
                <img alt="" className="h-4 w-4" src={reaction.emojiUrl} />
              ) : (
                <span>{reaction.emoji}</span>
              )}
              <span>{reaction.count}</span>
            </button>
          ))}
        </div>
      ) : null}
      {canReply ? (
        <button
          className="mt-1.5 text-xs font-medium text-muted-foreground outline-hidden transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/60"
          data-testid="video-review-comment-reply"
          type="button"
          onClick={() => onReply(item)}
        >
          Reply
        </button>
      ) : null}
    </div>
  );
}
