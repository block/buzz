import { openUrl } from "@tauri-apps/plugin-opener";
import { Check, ChevronUp, Mic, MicOff, Volume2, VolumeX } from "lucide-react";
import * as React from "react";
import type { CSSProperties } from "react";

import { cn } from "@/shared/lib/cn";
import { isMacPlatform } from "@/shared/lib/platform";
import { Button } from "@/shared/ui/button";
import {
  Popover,
  PopoverAnchor,
  PopoverContent,
  PopoverTrigger,
} from "@/shared/ui/popover";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

type VoiceInputMode = "push_to_talk" | "voice_activity";

type MicControlsProps = {
  isMuted: boolean;
  onToggleMute: () => void;
  isPttMode: boolean;
  pttActive: boolean;
  micConnected: boolean;
  micLevel: number;
  onSelectVoiceInputMode: (mode: VoiceInputMode) => void | Promise<void>;
  audioDevices: MediaDeviceInfo[];
  selectedDeviceId: string;
  onSelectDevice: (id: string) => void;
  micGain: number;
  onGainChange: (value: number) => void;
};

const splitIconButtonClass = "h-12 w-auto shrink-0 rounded-r-none px-4 py-4";
const splitChevronButtonClass =
  "buzz-huddle-split-chevron group h-12 w-auto shrink-0 rounded-l-none px-2 py-4";
const MIC_PERMISSION_SETTINGS_URL =
  "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone";

type MicMeterBarStyle = CSSProperties & {
  "--buzz-huddle-meter-height": string;
};

const MIC_METER_IDLE_HEIGHT_REM = 0.25;
const MIC_METER_IDLE_HEIGHTS: [number, number, number] = [
  MIC_METER_IDLE_HEIGHT_REM,
  MIC_METER_IDLE_HEIGHT_REM,
  MIC_METER_IDLE_HEIGHT_REM,
];

function clamp01(value: number): number {
  return Math.min(1, Math.max(0, value));
}

function micMeterBarStyle(heightRem: number): MicMeterBarStyle {
  return { "--buzz-huddle-meter-height": `${heightRem}rem` };
}

function micMeterHeights(level: number): [number, number, number] {
  const normalized = clamp01(level);
  if (normalized <= 0.01) return MIC_METER_IDLE_HEIGHTS;

  return [
    MIC_METER_IDLE_HEIGHT_REM + normalized * 0.5,
    MIC_METER_IDLE_HEIGHT_REM + normalized * 0.875,
    MIC_METER_IDLE_HEIGHT_REM + normalized * 0.625,
  ];
}

function usePrefersReducedMotion(): boolean {
  const [prefersReducedMotion, setPrefersReducedMotion] = React.useState(false);

  React.useEffect(() => {
    const mediaQuery = window.matchMedia("(prefers-reduced-motion: reduce)");
    const syncReducedMotion = () => setPrefersReducedMotion(mediaQuery.matches);

    syncReducedMotion();
    mediaQuery.addEventListener("change", syncReducedMotion);

    return () => {
      mediaQuery.removeEventListener("change", syncReducedMotion);
    };
  }, []);

  return prefersReducedMotion;
}

export function MicControls({
  isMuted,
  onToggleMute,
  isPttMode,
  pttActive,
  micConnected,
  micLevel,
  onSelectVoiceInputMode,
  audioDevices,
  selectedDeviceId,
  onSelectDevice,
  micGain,
  onGainChange,
}: MicControlsProps) {
  const micUnavailable = !micConnected;
  const showMicMeter = micConnected && !isMuted;
  const isMac = isMacPlatform();
  const prefersReducedMotion = usePrefersReducedMotion();
  const pushToTalkShortcut = isMac ? "⌃Space" : "Ctrl+Space";
  const barHeights: [number, number, number] = prefersReducedMotion
    ? MIC_METER_IDLE_HEIGHTS
    : micMeterHeights(showMicMeter ? micLevel : 0);
  const [leftBarHeight, centerBarHeight, rightBarHeight] = barHeights;

  const micButtonLabel = micUnavailable
    ? "Microphone unavailable"
    : isMuted
      ? "Unmute microphone"
      : isPttMode
        ? "Force mute (overrides PTT)"
        : "Mute microphone";

  return (
    <Popover>
      <div
        className={cn(
          "flex items-center rounded-md",
          isPttMode &&
            pttActive &&
            !isMuted &&
            micConnected &&
            "ring-2 ring-green-500 ring-offset-1 ring-offset-background",
        )}
      >
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              aria-disabled={micUnavailable}
              aria-label={micButtonLabel}
              aria-pressed={micConnected ? isMuted : true}
              className={cn(
                splitIconButtonClass,
                !isMuted && !micUnavailable && "buzz-huddle-split-main",
              )}
              onClick={() => {
                if (!micConnected) return;
                onToggleMute();
              }}
              size="icon"
              variant={isMuted || micUnavailable ? "destructive" : "secondary"}
            >
              {isMuted || micUnavailable ? (
                <MicOff className="h-4 w-4" />
              ) : (
                <Mic className="h-4 w-4" />
              )}
            </Button>
          </TooltipTrigger>
          <TooltipContent className="buzz-huddle-tooltip" side="top">
            {micUnavailable
              ? "Microphone unavailable. Check app permissions or input device."
              : micButtonLabel}
          </TooltipContent>
        </Tooltip>
        <PopoverTrigger asChild>
          <Button
            aria-label="Audio settings"
            className={splitChevronButtonClass}
            size="icon"
            variant="secondary"
          >
            {showMicMeter ? (
              <span
                aria-hidden="true"
                className="relative flex h-5 w-5 items-center justify-center"
              >
                <span className="flex h-5 w-5 items-center justify-between text-current group-data-[state=open]:hidden group-focus-visible:hidden group-hover:hidden">
                  <span
                    className="buzz-huddle-mic-meter-bar bg-current"
                    style={micMeterBarStyle(leftBarHeight)}
                  />
                  <span
                    className="buzz-huddle-mic-meter-bar bg-current"
                    style={micMeterBarStyle(centerBarHeight)}
                  />
                  <span
                    className="buzz-huddle-mic-meter-bar bg-current"
                    style={micMeterBarStyle(rightBarHeight)}
                  />
                </span>
                <ChevronUp className="hidden h-4 w-4 group-data-[state=open]:block group-focus-visible:block group-hover:block" />
              </span>
            ) : (
              <ChevronUp className="h-4 w-4" />
            )}
          </Button>
        </PopoverTrigger>
      </div>
      <PopoverContent
        side="top"
        className="buzz-huddle-drawer buzz-huddle-popover w-64 text-foreground"
      >
        <div className="flex flex-col gap-3">
          <div>
            <span className="mb-1 block text-xs font-medium">Input Mode</span>
            <button
              aria-label={
                isPttMode ? "Turn off Push to Talk" : "Turn on Push to Talk"
              }
              aria-pressed={isPttMode}
              className="flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-xs transition-colors hover:bg-accent"
              onClick={() =>
                void onSelectVoiceInputMode(
                  isPttMode ? "voice_activity" : "push_to_talk",
                )
              }
              type="button"
            >
              <Check
                className={cn("h-3 w-3 shrink-0", !isPttMode && "invisible")}
              />
              <span className="font-medium">Push to Talk</span>
              <kbd className="ml-auto rounded border border-foreground/10 px-1.5 py-0.5 text-2xs font-medium text-foreground/60">
                {pushToTalkShortcut}
              </kbd>
            </button>
            <span className="sr-only" aria-live="polite">
              {isPttMode
                ? "Push to Talk is enabled."
                : "Microphone is continuous."}
            </span>
          </div>
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
            {micUnavailable && (
              <div className="mt-3 rounded-md border border-foreground/10 bg-foreground/8 px-2 py-2 text-xs text-foreground">
                <p className="font-medium">Microphone unavailable</p>
                <p className="mt-1 leading-snug text-foreground/70">
                  Check app microphone permission or select another input
                  device.
                </p>
                {isMac && (
                  <Button
                    className="mt-2 h-7 border-foreground/15 bg-foreground/10 px-2 text-xs text-foreground hover:bg-foreground/15"
                    onClick={() => {
                      void openUrl(MIC_PERMISSION_SETTINGS_URL).catch(
                        (error) => {
                          console.error(
                            "[huddle] Failed to open microphone settings:",
                            error,
                          );
                        },
                      );
                    }}
                    size="sm"
                    type="button"
                    variant="secondary"
                  >
                    Open Settings
                  </Button>
                )}
              </div>
            )}
          </div>
        </div>
      </PopoverContent>
    </Popover>
  );
}

type SpeakerControlsProps = {
  ttsEnabled: boolean;
  showHeadphonesHint?: boolean;
  onHeadphonesHintDismiss?: () => void;
  onToggleTts: () => void;
  outputDevices: { name: string; is_default: boolean }[];
  selectedOutputDevice: string;
  onSelectOutputDevice: (name: string) => void;
};

export function SpeakerControls({
  ttsEnabled,
  showHeadphonesHint = false,
  onHeadphonesHintDismiss,
  onToggleTts,
  outputDevices,
  selectedOutputDevice,
  onSelectOutputDevice,
}: SpeakerControlsProps) {
  return (
    <Popover>
      <div className="relative flex items-center">
        <Popover
          open={showHeadphonesHint}
          onOpenChange={(open) => {
            if (!open) onHeadphonesHintDismiss?.();
          }}
        >
          <PopoverAnchor asChild>
            <span
              aria-hidden="true"
              className="pointer-events-none absolute inset-0"
            />
          </PopoverAnchor>
          <PopoverContent
            align="center"
            aria-label="Headphones recommended"
            className="buzz-huddle-drawer buzz-huddle-popover buzz-huddle-headphones-hint w-64 p-3 text-foreground"
            onCloseAutoFocus={(event) => event.preventDefault()}
            onOpenAutoFocus={(event) => event.preventDefault()}
            side="top"
            sideOffset={10}
          >
            <div className="flex flex-col gap-2">
              <div>
                <p className="text-xs font-medium">
                  Headphones help prevent echo
                </p>
                <p className="mt-1 text-xs leading-snug text-foreground/75">
                  If people are nearby, speakers can feed back into your mic.
                  Headphones keep huddles clearer.
                </p>
              </div>
              <div className="flex justify-end">
                <Button
                  className="h-7 bg-foreground/10 px-2 text-xs hover:bg-foreground/15"
                  onClick={onHeadphonesHintDismiss}
                  size="sm"
                  type="button"
                  variant="ghost"
                >
                  Got it
                </Button>
              </div>
            </div>
          </PopoverContent>
        </Popover>
        <Button
          aria-label={ttsEnabled ? "Mute agent speech" : "Unmute agent speech"}
          aria-pressed={!ttsEnabled}
          className={cn(
            splitIconButtonClass,
            ttsEnabled && "buzz-huddle-split-main",
          )}
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
            className={splitChevronButtonClass}
            size="icon"
            variant="secondary"
          >
            <ChevronUp className="h-4 w-4" />
          </Button>
        </PopoverTrigger>
      </div>
      <PopoverContent
        side="top"
        className="buzz-huddle-drawer buzz-huddle-popover w-64 text-foreground"
      >
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
  const seenDeviceIds = new Map<string, number>();
  const keyedDevices = devices.map((device) => {
    const occurrence = (seenDeviceIds.get(device.id) ?? 0) + 1;
    seenDeviceIds.set(device.id, occurrence);

    return {
      ...device,
      key: occurrence === 1 ? device.id : `${device.id}-${occurrence}`,
    };
  });

  return (
    <div>
      <span className="mb-1 block text-xs font-medium">{label}</span>
      <ul className="flex flex-col">
        <li>
          <button
            className="flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-xs transition-colors hover:bg-accent"
            onClick={() => onSelect("")}
            type="button"
          >
            <Check
              className={cn("h-4 w-4 shrink-0", selectedId && "invisible")}
            />
            System default
          </button>
        </li>
        {keyedDevices.map((d) => {
          const isSelected = selectedId === d.id;
          return (
            <li key={d.key}>
              <button
                className="flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-xs transition-colors hover:bg-accent"
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
        <p className="mt-1 text-2xs text-muted-foreground">
          Change takes effect on next huddle
        </p>
      )}
    </div>
  );
}
