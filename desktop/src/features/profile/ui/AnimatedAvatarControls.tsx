import { Circle, CircleDashed } from "lucide-react";
import * as React from "react";

import { clampFrameIndex } from "@/features/profile/ui/AnimatedAvatarCapture.helpers";
import { cn } from "@/shared/lib/cn";
import { performDefaultHaptic } from "@/shared/lib/haptics";
import { Spinner } from "@/shared/ui/spinner";

const FILMSTRIP_SELECTOR_SIZE = 48;
const SLIDER_TICK_STEP = 10;

function percentFromSliderValue(
  value: number,
  min: number,
  max: number,
): number {
  if (max === min) {
    return 0;
  }
  return ((value - min) / (max - min)) * 100;
}

function buildAnchoredSliderTicks(
  min: number,
  max: number,
  resetValue: number,
): number[] {
  const ticks = new Set<number>();
  for (let tick = resetValue; tick >= min; tick -= SLIDER_TICK_STEP) {
    ticks.add(Math.round(tick));
  }
  for (
    let tick = resetValue + SLIDER_TICK_STEP;
    tick <= max;
    tick += SLIDER_TICK_STEP
  ) {
    ticks.add(Math.round(tick));
  }
  return [...ticks].sort((first, second) => first - second);
}

function findCrossedSliderTick(
  previousValue: number,
  nextValue: number,
  ticks: number[],
): number | null {
  if (previousValue === nextValue) {
    return null;
  }

  if (nextValue > previousValue) {
    return (
      ticks.find((tick) => tick > previousValue && tick <= nextValue) ?? null
    );
  }

  return (
    [...ticks]
      .reverse()
      .find((tick) => tick < previousValue && tick >= nextValue) ?? null
  );
}

type AvatarFramingSliderProps = {
  disabled?: boolean;
  helpText?: string | null;
  helpTestId?: string;
  max: number;
  min: number;
  onChange: (value: number) => void;
  onReset: () => void;
  resetValue: number;
  resetTestId: string;
  testId: string;
  tipText?: string | null;
  value: number;
};

export function AvatarFramingSlider({
  disabled = false,
  helpText = null,
  helpTestId,
  max,
  min,
  onChange,
  onReset,
  resetValue,
  resetTestId,
  testId,
  tipText = null,
  value,
}: AvatarFramingSliderProps) {
  const sliderRef = React.useRef<HTMLDivElement | null>(null);
  const activePointerRef = React.useRef<number | null>(null);
  const valueRef = React.useRef(value);
  const lastHapticTickRef = React.useRef<number | null>(null);
  const [isHovered, setIsHovered] = React.useState(false);
  const [isFocused, setIsFocused] = React.useState(false);
  const [isInteracting, setIsInteracting] = React.useState(false);
  const fill = percentFromSliderValue(value, min, max);
  const isActive = isHovered || isFocused || isInteracting;
  const tipId = React.useId();
  const ticks = React.useMemo(
    () => buildAnchoredSliderTicks(min, max, resetValue),
    [max, min, resetValue],
  );

  React.useEffect(() => {
    valueRef.current = value;
  }, [value]);

  const commitValue = React.useCallback(
    (nextValue: number) => {
      const clampedValue = Math.min(max, Math.max(min, Math.round(nextValue)));
      const crossedTick = findCrossedSliderTick(
        valueRef.current,
        clampedValue,
        ticks,
      );
      valueRef.current = clampedValue;
      onChange(clampedValue);
      if (crossedTick !== null && crossedTick !== lastHapticTickRef.current) {
        lastHapticTickRef.current = crossedTick;
        performDefaultHaptic();
      } else if (crossedTick === null) {
        lastHapticTickRef.current = null;
      }
    },
    [max, min, onChange, ticks],
  );

  const commitPointerValue = React.useCallback(
    (clientX: number) => {
      const slider = sliderRef.current;
      if (!slider) {
        return;
      }
      const rect = slider.getBoundingClientRect();
      const progress = Math.min(
        1,
        Math.max(0, (clientX - rect.left) / Math.max(rect.width, 1)),
      );
      commitValue(min + progress * (max - min));
    },
    [commitValue, max, min],
  );

  const nudgeValue = React.useCallback(
    (delta: number) => {
      commitValue(value + delta);
    },
    [commitValue, value],
  );

  const resetTickStyle = {
    left: `${percentFromSliderValue(resetValue, min, max)}%`,
  };
  const sliderControl = (
    <div className="buzz-avatar-framing-slider-wrapper">
      <div
        aria-label="Avatar size"
        aria-describedby={tipText ? tipId : undefined}
        aria-valuemax={max}
        aria-valuemin={min}
        aria-valuenow={value}
        className="buzz-avatar-framing-slider"
        data-active={isActive ? "true" : undefined}
        data-testid={testId}
        onKeyDown={(event) => {
          if (disabled) {
            return;
          }
          const step = event.shiftKey ? 10 : 1;
          if (event.key === "ArrowLeft" || event.key === "ArrowDown") {
            event.preventDefault();
            nudgeValue(-step);
          } else if (event.key === "ArrowRight" || event.key === "ArrowUp") {
            event.preventDefault();
            nudgeValue(step);
          } else if (event.key === "Home") {
            event.preventDefault();
            commitValue(min);
          } else if (event.key === "End") {
            event.preventDefault();
            commitValue(max);
          }
        }}
        onBlur={() => setIsFocused(false)}
        onFocus={() => setIsFocused(true)}
        onMouseEnter={() => setIsHovered(true)}
        onMouseLeave={() => setIsHovered(false)}
        onPointerCancel={(event) => {
          if (activePointerRef.current === event.pointerId) {
            activePointerRef.current = null;
            setIsInteracting(false);
          }
        }}
        onPointerDown={(event) => {
          if (disabled) {
            return;
          }
          event.preventDefault();
          event.currentTarget.setPointerCapture(event.pointerId);
          activePointerRef.current = event.pointerId;
          setIsInteracting(true);
          commitPointerValue(event.clientX);
        }}
        onPointerMove={(event) => {
          if (activePointerRef.current !== event.pointerId) {
            return;
          }
          commitPointerValue(event.clientX);
        }}
        onPointerUp={(event) => {
          if (activePointerRef.current === event.pointerId) {
            activePointerRef.current = null;
            setIsInteracting(false);
          }
        }}
        ref={sliderRef}
        role="slider"
        style={
          {
            "--buzz-avatar-framing-slider-fill": `${fill}%`,
          } as React.CSSProperties
        }
        tabIndex={disabled ? -1 : 0}
      >
        <div className="buzz-avatar-framing-slider-hashmarks">
          {ticks.map((tick) => (
            <span
              aria-hidden="true"
              className="buzz-avatar-framing-slider-hashmark"
              key={tick}
              style={{
                left: `${percentFromSliderValue(tick, min, max)}%`,
              }}
            />
          ))}
        </div>
        <div aria-hidden="true" className="buzz-avatar-framing-slider-fill" />
        <div aria-hidden="true" className="buzz-avatar-framing-slider-handle" />
      </div>
      <button
        aria-label="Reset avatar size"
        className="buzz-avatar-framing-slider-hashmark"
        data-reset="true"
        data-testid={resetTestId}
        disabled={disabled}
        onClick={(event) => {
          event.preventDefault();
          valueRef.current = resetValue;
          lastHapticTickRef.current = resetValue;
          performDefaultHaptic();
          onReset();
        }}
        style={resetTickStyle}
        title="Reset avatar size"
        type="button"
      />
      {tipText ? (
        <p
          className="buzz-avatar-framing-slider-tip"
          data-visible={isActive ? "true" : undefined}
          id={tipId}
        >
          {tipText}
        </p>
      ) : null}
    </div>
  );

  return helpText ? (
    <div className="grid gap-2">
      {sliderControl}
      <p
        className="px-1 text-center text-sm text-muted-foreground"
        data-testid={helpTestId}
      >
        {helpText}
      </p>
    </div>
  ) : (
    sliderControl
  );
}

type AvatarOutlineToggleProps = {
  disabled?: boolean;
  enabled: boolean;
  onChange: (enabled: boolean) => void;
  testIdPrefix: string;
};

export function AvatarOutlineToggle({
  disabled = false,
  enabled,
  onChange,
  testIdPrefix,
}: AvatarOutlineToggleProps) {
  const Icon = enabled ? Circle : CircleDashed;
  return (
    <button
      aria-label={enabled ? "Turn outline off" : "Turn outline on"}
      aria-pressed={enabled}
      className={cn(
        "grid h-12 w-12 shrink-0 place-items-center rounded-full border border-foreground/10 bg-background text-foreground transition-[background-color,box-shadow,color] duration-150 ease-out hover:bg-muted/70 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
        enabled ? "shadow-xs" : "text-muted-foreground",
      )}
      data-testid={`${testIdPrefix}-animated-outline-toggle`}
      disabled={disabled}
      onClick={() => onChange(!enabled)}
      title={enabled ? "Outline on" : "Outline off"}
      type="button"
    >
      <Icon aria-hidden="true" className="h-4 w-4" />
    </button>
  );
}

type AvatarFilmstripPickerProps = {
  disabled?: boolean;
  frameCount: number;
  frames: string[];
  helpText?: string;
  helpTestId?: string;
  onSelectFrame: (index: number) => void;
  selectedFrame: number;
  testIdPrefix: string;
};

export function AvatarFilmstripPicker({
  disabled = false,
  frameCount,
  frames,
  helpText,
  helpTestId,
  onSelectFrame,
  selectedFrame,
  testIdPrefix,
}: AvatarFilmstripPickerProps) {
  const stripRef = React.useRef<HTMLDivElement | null>(null);
  const maxFrameIndex = Math.max(0, frameCount - 1);
  const safeSelectedFrame = clampFrameIndex(selectedFrame, frameCount);
  const selectedFrameProgress =
    maxFrameIndex === 0 ? 0 : safeSelectedFrame / maxFrameIndex;

  const selectFromClientX = React.useCallback(
    (clientX: number) => {
      if (disabled) {
        return;
      }

      const strip = stripRef.current;
      if (!strip) {
        return;
      }

      const rect = strip.getBoundingClientRect();
      const nextProgress = Math.min(
        1,
        Math.max(0, (clientX - rect.left) / Math.max(rect.width, 1)),
      );
      onSelectFrame(Math.round(nextProgress * maxFrameIndex));
    },
    [disabled, maxFrameIndex, onSelectFrame],
  );

  const nudge = React.useCallback(
    (delta: number) => {
      if (disabled) {
        return;
      }
      onSelectFrame(clampFrameIndex(safeSelectedFrame + delta, frameCount));
    },
    [disabled, frameCount, onSelectFrame, safeSelectedFrame],
  );

  return (
    <div
      className="grid gap-2"
      data-testid={`${testIdPrefix}-animated-poster-strip`}
    >
      <div
        aria-label="Choose still frame"
        aria-valuemax={maxFrameIndex}
        aria-valuemin={0}
        aria-valuenow={safeSelectedFrame}
        className="relative h-12 min-w-0 touch-none rounded-md focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        data-testid={`${testIdPrefix}-animated-poster-scrubber`}
        onKeyDown={(event) => {
          if (event.key === "ArrowLeft") {
            event.preventDefault();
            nudge(event.shiftKey ? -3 : -1);
          } else if (event.key === "ArrowRight") {
            event.preventDefault();
            nudge(event.shiftKey ? 3 : 1);
          } else if (event.key === "Home") {
            event.preventDefault();
            onSelectFrame(0);
          } else if (event.key === "End") {
            event.preventDefault();
            onSelectFrame(maxFrameIndex);
          }
        }}
        onPointerDown={(event) => {
          if (disabled) {
            return;
          }
          event.preventDefault();
          event.currentTarget.setPointerCapture(event.pointerId);
          selectFromClientX(event.clientX);
        }}
        onPointerMove={(event) => {
          if (event.buttons !== 1) {
            return;
          }
          selectFromClientX(event.clientX);
        }}
        ref={stripRef}
        role="slider"
        tabIndex={disabled ? -1 : 0}
      >
        <div className="absolute inset-0 overflow-hidden rounded-md border border-foreground/10 bg-background/70 shadow-inner">
          {frames.length === 0 ? (
            <div className="grid h-full w-full place-items-center">
              <Spinner
                aria-label="Generating frame thumbnails"
                className="h-5 w-5"
              />
            </div>
          ) : (
            <div aria-hidden="true" className="absolute inset-0 flex h-full">
              {frames.map((frame, index) => (
                <img
                  alt=""
                  className="h-full min-w-0 flex-1 object-cover"
                  draggable={false}
                  // biome-ignore lint/suspicious/noArrayIndexKey: filmstrip frames are regenerated as a fixed, ordered capture sequence.
                  key={`filmstrip-frame-${index}`}
                  src={frame}
                />
              ))}
            </div>
          )}
        </div>
        {frames.length > 0 ? (
          <div
            aria-hidden="true"
            className="pointer-events-none absolute top-1/2 z-10 h-12 w-12 -translate-x-1/2 -translate-y-1/2 rounded-lg border-[3px] border-white bg-white/10 shadow-[0_4px_16px_rgba(0,0,0,0.32)] ring-1 ring-black/20 transition-[left] duration-75 ease-out"
            data-testid={`${testIdPrefix}-animated-poster-selector`}
            style={{
              left: `clamp(${FILMSTRIP_SELECTOR_SIZE / 2}px, ${
                selectedFrameProgress * 100
              }%, calc(100% - ${FILMSTRIP_SELECTOR_SIZE / 2}px))`,
            }}
          />
        ) : null}
      </div>
      {helpText ? (
        <p
          className="px-1 text-center text-sm text-muted-foreground"
          data-testid={helpTestId}
        >
          {helpText}
        </p>
      ) : null}
    </div>
  );
}
