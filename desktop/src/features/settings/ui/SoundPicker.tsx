import { useEffect, useRef, useState } from "react";
import { ChevronDown, Pause, Play } from "lucide-react";

import {
  createNotificationSound,
  SOUND_NAMES,
  type SoundName,
} from "@/features/notifications/lib/sound";
import { cn } from "@/shared/lib/cn";
import { SettingsActionButton as Button } from "./SettingsActionButton";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";

function sortedSounds(recommended: SoundName): SoundName[] {
  const others = SOUND_NAMES.filter((n) => n !== recommended)
    .slice()
    .sort();
  return [recommended, ...others];
}

// The waveform SVGs use fill="currentColor", which an <img> can't inherit,
// so render them as a mask over the current text color instead.
function Waveform({
  name,
  className,
  progress,
}: {
  name: SoundName;
  className?: string;
  progress?: number;
}) {
  const maskImage = `url(/sounds/${name}.svg)`;
  const maskStyle = {
    maskImage,
    maskPosition: "center",
    maskRepeat: "no-repeat",
    maskSize: "contain",
    WebkitMaskImage: maskImage,
    WebkitMaskPosition: "center",
    WebkitMaskRepeat: "no-repeat",
    WebkitMaskSize: "contain",
  } as const;

  if (progress === undefined) {
    return (
      <span
        aria-hidden="true"
        className={cn("inline-block shrink-0 bg-current", className)}
        style={maskStyle}
      />
    );
  }

  const clampedProgress = Math.max(0, Math.min(1, progress));
  return (
    <span
      aria-hidden="true"
      className={cn("relative inline-block shrink-0", className)}
      data-playback-progress={clampedProgress.toFixed(3)}
      data-testid="sound-picker-waveform"
    >
      <span
        className="absolute inset-0 bg-current opacity-25"
        style={maskStyle}
      />
      <span
        className="absolute inset-0 bg-current transition-[clip-path] duration-75 ease-linear motion-reduce:transition-none"
        style={{
          ...maskStyle,
          clipPath: `inset(0 ${(1 - clampedProgress) * 100}% 0 0)`,
        }}
      />
    </span>
  );
}

export function SoundPicker({
  recommended,
  value,
  disabled,
  onChange,
}: {
  recommended: SoundName;
  value: SoundName;
  disabled?: boolean;
  onChange: (next: SoundName) => void;
}) {
  const items = sortedSounds(recommended);
  const [isPlaying, setIsPlaying] = useState(false);
  const [playbackProgress, setPlaybackProgress] = useState(0);
  const audioRef = useRef<HTMLAudioElement | null>(null);
  const animationFrameRef = useRef<number | null>(null);

  useEffect(() => {
    return () => {
      if (animationFrameRef.current !== null) {
        cancelAnimationFrame(animationFrameRef.current);
      }
      audioRef.current?.pause();
      audioRef.current = null;
    };
  }, []);

  function stopPreview() {
    if (animationFrameRef.current !== null) {
      cancelAnimationFrame(animationFrameRef.current);
      animationFrameRef.current = null;
    }
    const audio = audioRef.current;
    audioRef.current = null;
    audio?.pause();
    if (audio) audio.currentTime = 0;
    setIsPlaying(false);
    setPlaybackProgress(0);
  }

  function handleChange(next: SoundName) {
    stopPreview();
    onChange(next);
  }

  function togglePreview() {
    if (isPlaying) {
      stopPreview();
      return;
    }

    const audio = createNotificationSound(value);
    audio.preload = "auto";
    audioRef.current = audio;
    setPlaybackProgress(0);
    setIsPlaying(true);

    const updateProgress = () => {
      if (audioRef.current !== audio) return;
      const duration = audio.duration;
      if (Number.isFinite(duration) && duration > 0) {
        setPlaybackProgress(Math.min(1, audio.currentTime / duration));
      }
      if (audioRef.current === audio && !audio.ended) {
        animationFrameRef.current = requestAnimationFrame(updateProgress);
      }
    };
    const finish = () => {
      if (audioRef.current !== audio) return;
      if (animationFrameRef.current !== null) {
        cancelAnimationFrame(animationFrameRef.current);
        animationFrameRef.current = null;
      }
      audioRef.current = null;
      setPlaybackProgress(1);
      setIsPlaying(false);
    };
    const fail = () => {
      if (audioRef.current !== audio) return;
      audioRef.current = null;
      setIsPlaying(false);
      setPlaybackProgress(0);
    };

    audio.addEventListener("ended", finish, { once: true });
    audio.addEventListener("error", fail, { once: true });
    void audio.play().then(() => {
      if (audioRef.current === audio) {
        animationFrameRef.current = requestAnimationFrame(updateProgress);
      }
    }, fail);
  }

  return (
    <span
      className="inline-flex items-center gap-1.5"
      data-testid="sound-picker"
    >
      <DropdownMenu modal={false}>
        <DropdownMenuTrigger asChild>
          <Button
            className="h-7 min-w-40 justify-between gap-1.5 rounded-full border border-border/50 bg-muted/45 px-2.5 text-xs font-medium text-foreground shadow-none hover:bg-muted/70"
            disabled={disabled}
            size="sm"
            type="button"
            variant="ghost"
          >
            <span className="truncate">{value}</span>
            <span className="flex items-center gap-1.5">
              <Waveform
                className="h-6 w-15 opacity-70"
                name={value}
                progress={
                  isPlaying || playbackProgress > 0
                    ? playbackProgress
                    : undefined
                }
              />
              <ChevronDown className="h-4 w-4 text-muted-foreground" />
            </span>
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent
          align="end"
          className="max-h-80 min-w-72 overflow-y-auto"
        >
          <DropdownMenuRadioGroup
            onValueChange={(next) => handleChange(next as SoundName)}
            value={value}
          >
            {items.map((name) => (
              <DropdownMenuRadioItem key={name} value={name}>
                <span className="flex w-full items-center justify-between gap-3">
                  <span>{name}</span>
                  <span className="flex items-center gap-2">
                    {name === recommended ? (
                      <span className="text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
                        rec.
                      </span>
                    ) : null}
                    <Waveform className="h-6 w-15 opacity-70" name={name} />
                  </span>
                </span>
              </DropdownMenuRadioItem>
            ))}
          </DropdownMenuRadioGroup>
        </DropdownMenuContent>
      </DropdownMenu>
      <Button
        aria-label={isPlaying ? `Pause ${value}` : `Preview ${value}`}
        className="h-7 w-7 rounded-full border border-border/50 bg-muted/45 p-0 text-foreground shadow-none hover:bg-muted/70"
        disabled={disabled}
        onClick={togglePreview}
        size="sm"
        type="button"
        variant="ghost"
      >
        {isPlaying ? (
          <Pause className="h-4 w-4" />
        ) : (
          <Play className="h-4 w-4" />
        )}
      </Button>
    </span>
  );
}
