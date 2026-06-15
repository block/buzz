import { Check, ChevronUp, Mic, MicOff, Volume2, VolumeX } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";

type MicControlsProps = {
  isMuted: boolean;
  onToggleMute: () => void;
  isPttMode: boolean;
  pttActive: boolean;
  micConnected: boolean;
  audioDevices: MediaDeviceInfo[];
  selectedDeviceId: string;
  onSelectDevice: (id: string) => void;
  micGain: number;
  onGainChange: (value: number) => void;
};

export function MicControls({
  isMuted,
  onToggleMute,
  isPttMode,
  pttActive,
  micConnected,
  audioDevices,
  selectedDeviceId,
  onSelectDevice,
  micGain,
  onGainChange,
}: MicControlsProps) {
  return (
    <Popover>
      <div
        className={cn(
          "flex items-center rounded-md",
          isPttMode &&
            pttActive &&
            !isMuted &&
            "ring-2 ring-green-500 ring-offset-1 ring-offset-background",
        )}
      >
        <Button
          aria-label={
            isMuted
              ? "Unmute microphone"
              : isPttMode
                ? "Force mute (overrides PTT)"
                : "Mute microphone"
          }
          aria-pressed={isMuted}
          className="h-8 w-8 rounded-r-none"
          onClick={onToggleMute}
          size="icon"
          variant={isMuted ? "destructive" : "secondary"}
        >
          {isMuted ? (
            <MicOff className="h-4 w-4" />
          ) : (
            <Mic className="h-4 w-4" />
          )}
        </Button>
        <PopoverTrigger asChild>
          <Button
            aria-label="Audio settings"
            className="h-8 w-5 rounded-l-none border-l px-0"
            size="icon"
            variant="secondary"
          >
            <ChevronUp className="h-4 w-4" />
          </Button>
        </PopoverTrigger>
      </div>
      <PopoverContent side="top" className="w-64">
        <div className="flex flex-col gap-3">
          <DeviceList
            label="Microphone"
            devices={audioDevices.map((d) => ({
              id: d.deviceId,
              label: d.label || `Mic ${d.deviceId.slice(0, 8)}`,
            }))}
            selectedId={selectedDeviceId}
            onSelect={onSelectDevice}
            showChangeHint={!!selectedDeviceId && micConnected}
          />
          <div>
            <label
              htmlFor="mic-volume"
              className="mb-1 block text-xs font-medium"
            >
              Input Volume
            </label>
            <div className="flex items-center gap-2">
              <input
                id="mic-volume"
                type="range"
                min={0}
                max={1}
                step={0.01}
                value={micGain}
                onChange={(e) => onGainChange(Number(e.target.value))}
                className="h-1.5 w-full cursor-pointer appearance-none rounded-full bg-muted accent-foreground"
              />
              <span className="w-8 text-right text-xs text-muted-foreground">
                {Math.round(micGain * 100)}%
              </span>
            </div>
          </div>
        </div>
      </PopoverContent>
    </Popover>
  );
}

type SpeakerControlsProps = {
  ttsEnabled: boolean;
  onToggleTts: () => void;
  outputDevices: { name: string; is_default: boolean }[];
  selectedOutputDevice: string;
  onSelectOutputDevice: (name: string) => void;
};

export function SpeakerControls({
  ttsEnabled,
  onToggleTts,
  outputDevices,
  selectedOutputDevice,
  onSelectOutputDevice,
}: SpeakerControlsProps) {
  return (
    <Popover>
      <div className="flex items-center">
        <Button
          aria-label={ttsEnabled ? "Mute agent speech" : "Unmute agent speech"}
          aria-pressed={!ttsEnabled}
          className="h-8 w-8 rounded-r-none"
          onClick={onToggleTts}
          size="icon"
          variant={ttsEnabled ? "secondary" : "destructive"}
        >
          {ttsEnabled ? (
            <Volume2 className="h-4 w-4" />
          ) : (
            <VolumeX className="h-4 w-4" />
          )}
        </Button>
        <PopoverTrigger asChild>
          <Button
            aria-label="Speaker settings"
            className="h-8 w-5 rounded-l-none border-l px-0"
            size="icon"
            variant="secondary"
          >
            <ChevronUp className="h-4 w-4" />
          </Button>
        </PopoverTrigger>
      </div>
      <PopoverContent side="top" className="w-64">
        <DeviceList
          label="Speaker"
          devices={outputDevices.map((d) => ({ id: d.name, label: d.name }))}
          selectedId={selectedOutputDevice}
          onSelect={onSelectOutputDevice}
          showChangeHint={!!selectedOutputDevice}
        />
      </PopoverContent>
    </Popover>
  );
}

export function DeviceList({
  label,
  devices,
  selectedId,
  onSelect,
  showChangeHint,
}: {
  label: string;
  devices: { id: string; label: string }[];
  selectedId: string;
  onSelect: (id: string) => void;
  showChangeHint: boolean;
}) {
  return (
    <div>
      <span className="mb-1 block text-xs font-medium">{label}</span>
      <ul className="flex flex-col">
        <li>
          <button
            className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs transition-colors hover:bg-accent"
            onClick={() => onSelect("")}
            type="button"
          >
            <Check
              className={cn("h-4 w-4 shrink-0", selectedId && "invisible")}
            />
            System default
          </button>
        </li>
        {devices.map((d) => {
          const isSelected = selectedId === d.id;
          return (
            <li key={d.id}>
              <button
                className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs transition-colors hover:bg-accent"
                onClick={() => onSelect(d.id)}
                type="button"
              >
                <Check
                  className={cn("h-4 w-4 shrink-0", !isSelected && "invisible")}
                />
                <span className="truncate">{d.label}</span>
              </button>
            </li>
          );
        })}
      </ul>
      {showChangeHint && (
        <p className="mt-1 text-[10px] text-muted-foreground">
          Change takes effect on next huddle
        </p>
      )}
    </div>
  );
}
