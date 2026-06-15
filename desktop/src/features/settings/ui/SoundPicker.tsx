import { useRef, useState } from "react";
import { ChevronDown, Pause, Play } from "lucide-react";

import {
  playNotificationSound,
  SOUND_NAMES,
  type SoundName,
} from "@/features/notifications/lib/sound";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
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
}: {
  name: SoundName;
  className?: string;
}) {
  const maskImage = `url(/sounds/${name}.svg)`;
  return (
    <span
      aria-hidden="true"
      className={cn("inline-block shrink-0 bg-current", className)}
      style={{
        maskImage,
        maskPosition: "center",
        maskRepeat: "no-repeat",
        maskSize: "contain",
        WebkitMaskImage: maskImage,
        WebkitMaskPosition: "center",
        WebkitMaskRepeat: "no-repeat",
        WebkitMaskSize: "contain",
      }}
    />
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
  const audioRef = useRef<HTMLAudioElement | null>(null);

  function togglePreview() {
    if (isPlaying) {
      audioRef.current?.pause();
      setIsPlaying(false);
      return;
    }
    const audio = playNotificationSound(value);
    if (!audio) return;
    audioRef.current = audio;
    setIsPlaying(true);
    const stop = () => setIsPlaying(false);
    audio.addEventListener("ended", stop, { once: true });
    audio.addEventListener("pause", stop, { once: true });
  }

  return (
    <span className="inline-flex items-center gap-1.5">
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
              <Waveform className="h-6 w-15 opacity-70" name={value} />
              <ChevronDown className="h-3 w-3 text-muted-foreground" />
            </span>
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent
          align="end"
          className="max-h-80 min-w-72 overflow-y-auto"
        >
          <DropdownMenuRadioGroup
            onValueChange={(next) => onChange(next as SoundName)}
            value={value}
          >
            {items.map((name) => (
              <DropdownMenuRadioItem key={name} value={name}>
                <span className="flex w-full items-center justify-between gap-3">
                  <span>{name}</span>
                  <span className="flex items-center gap-2">
                    {name === recommended ? (
                      <span className="text-[10px] uppercase tracking-wide text-muted-foreground">
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
