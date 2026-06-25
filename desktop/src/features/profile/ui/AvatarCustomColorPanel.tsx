import { motion } from "motion/react";
import * as React from "react";

import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import {
  CUSTOM_COLOR_GRID_COLUMNS,
  CUSTOM_COLOR_GRID_HORIZONTAL_INSET,
  CUSTOM_COLOR_GRID_ROWS,
  CUSTOM_COLOR_GRID_VERTICAL_INSET,
  CUSTOM_HUE_SCRUBBER_INSET,
  clampPercent,
  gridInsetPosition,
  hueScrubberPosition,
  normalizeHue,
  snapToGrid,
} from "./ProfileAvatarEditor.utils";

const PANEL_MOTION_TRANSITION = {
  duration: 0.25,
  ease: "easeOut",
} as const;

type AvatarCustomColorPanelProps = {
  visible: boolean;
  hue: number;
  /** 0-100 */
  saturation: number;
  /** 0-100 */
  value: number;
  /** Hex color derived from hue/saturation/value. */
  colorDraft: string;
  onHueChange: (hue: number) => void;
  onSaturationValueChange: (saturation: number, value: number) => void;
  onCommit: () => void;
  testIdPrefix: string;
  className?: string;
};

/**
 * The HSV custom color picker overlay shared by the emoji and animated
 * avatar tabs: a saturation/value spectrum grid, a hue scrubber, and a
 * commit button. The parent owns the HSV state and positions the panel
 * (it fills its nearest relative ancestor).
 */
export function AvatarCustomColorPanel({
  visible,
  hue,
  saturation,
  value,
  colorDraft,
  onHueChange,
  onSaturationValueChange,
  onCommit,
  testIdPrefix,
  className,
}: AvatarCustomColorPanelProps) {
  const hueDragUserSelectRef = React.useRef<string | null>(null);

  const unlockHueDragSelection = React.useCallback(() => {
    if (hueDragUserSelectRef.current === null) {
      return;
    }

    document.body.style.userSelect = hueDragUserSelectRef.current;
    hueDragUserSelectRef.current = null;
  }, []);

  const lockHueDragSelection = React.useCallback(() => {
    if (hueDragUserSelectRef.current !== null) {
      return;
    }

    hueDragUserSelectRef.current = document.body.style.userSelect;
    document.body.style.userSelect = "none";
  }, []);

  const updateColorFromPointer = React.useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      const rect = event.currentTarget.getBoundingClientRect();
      const width = Math.max(
        rect.width - CUSTOM_COLOR_GRID_HORIZONTAL_INSET * 2,
        1,
      );
      const height = Math.max(
        rect.height - CUSTOM_COLOR_GRID_VERTICAL_INSET * 2,
        1,
      );
      const rawSaturation = clampPercent(
        ((event.clientX - rect.left - CUSTOM_COLOR_GRID_HORIZONTAL_INSET) /
          width) *
          100,
      );
      const rawValue = clampPercent(
        (1 -
          (event.clientY - rect.top - CUSTOM_COLOR_GRID_VERTICAL_INSET) /
            height) *
          100,
      );
      const nextSaturation = Math.round(
        snapToGrid(rawSaturation, CUSTOM_COLOR_GRID_COLUMNS),
      );
      const nextValue = Math.round(
        snapToGrid(rawValue, CUSTOM_COLOR_GRID_ROWS),
      );

      onSaturationValueChange(nextSaturation, nextValue);
    },
    [onSaturationValueChange],
  );

  const updateHueFromPointer = React.useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      const rect = event.currentTarget.getBoundingClientRect();
      const trackWidth = Math.max(
        rect.width - CUSTOM_HUE_SCRUBBER_INSET * 2,
        1,
      );
      const nextPercent = clampPercent(
        ((event.clientX - rect.left - CUSTOM_HUE_SCRUBBER_INSET) / trackWidth) *
          100,
      );
      onHueChange(Math.round((nextPercent / 100) * 360));
    },
    [onHueChange],
  );

  const adjustHue = React.useCallback(
    (delta: number) => {
      onHueChange(normalizeHue(hue + delta));
    },
    [hue, onHueChange],
  );

  return (
    <motion.div
      aria-hidden={!visible}
      className={cn(
        "absolute inset-0 z-40 flex flex-col rounded-xl bg-muted p-4",
        visible ? "pointer-events-auto" : "pointer-events-none",
        className,
      )}
      animate={visible ? { opacity: 1 } : { opacity: 0 }}
      inert={visible ? undefined : true}
      initial={false}
      transition={PANEL_MOTION_TRANSITION}
    >
      <div
        className="relative min-h-0 w-full flex-1 cursor-pointer overflow-hidden rounded-xl shadow-[inset_0_-18px_34px_rgba(0,0,0,0.18)]"
        data-testid={`${testIdPrefix}-custom-color-spectrum`}
        onPointerDown={(event) => {
          event.preventDefault();
          event.currentTarget.setPointerCapture(event.pointerId);
          updateColorFromPointer(event);
        }}
        onPointerMove={(event) => {
          if (event.buttons === 1) {
            event.preventDefault();
            updateColorFromPointer(event);
          }
        }}
        onPointerUp={(event) => {
          event.preventDefault();
          updateColorFromPointer(event);
        }}
        style={{
          backgroundColor: `hsl(${hue}, 100%, 50%)`,
          backgroundImage:
            "linear-gradient(to bottom, transparent 0%, #000000 100%), linear-gradient(to right, #ffffff 0%, rgba(255,255,255,0) 100%)",
        }}
      >
        <div
          aria-hidden="true"
          className="pointer-events-none absolute"
          style={{
            inset: `${CUSTOM_COLOR_GRID_VERTICAL_INSET}px ${CUSTOM_COLOR_GRID_HORIZONTAL_INSET}px`,
          }}
        >
          {Array.from({
            length: CUSTOM_COLOR_GRID_COLUMNS * CUSTOM_COLOR_GRID_ROWS,
          }).map((_, index) => {
            const column = index % CUSTOM_COLOR_GRID_COLUMNS;
            const row = Math.floor(index / CUSTOM_COLOR_GRID_COLUMNS);
            const gridSaturation = Math.round(
              (column / (CUSTOM_COLOR_GRID_COLUMNS - 1)) * 100,
            );
            const gridValue = Math.round(
              100 - (row / (CUSTOM_COLOR_GRID_ROWS - 1)) * 100,
            );
            const isSelectedGridDot =
              gridSaturation === saturation && gridValue === value;

            return (
              <span
                className={cn(
                  "absolute h-1 w-1 -translate-x-1/2 -translate-y-1/2 rounded-full bg-white/60 shadow-[0_0_4px_rgba(255,255,255,0.24)]",
                  isSelectedGridDot &&
                    "h-3 w-3 border-2 border-white shadow-[0_2px_10px_rgba(0,0,0,0.24)]",
                )}
                key={`${column}-${row}`}
                style={{
                  backgroundColor: isSelectedGridDot ? colorDraft : undefined,
                  left: `${(column / (CUSTOM_COLOR_GRID_COLUMNS - 1)) * 100}%`,
                  top: `${(row / (CUSTOM_COLOR_GRID_ROWS - 1)) * 100}%`,
                }}
              />
            );
          })}
        </div>
        <div
          className="pointer-events-none absolute h-8 w-8 -translate-x-1/2 -translate-y-1/2 rounded-full border-[3px] border-white shadow-[0_5px_16px_rgba(0,0,0,0.24),inset_0_0_0_1px_rgba(0,0,0,0.06)]"
          style={{
            backgroundColor: colorDraft,
            left: gridInsetPosition(
              saturation,
              CUSTOM_COLOR_GRID_HORIZONTAL_INSET,
            ),
            top: gridInsetPosition(
              100 - value,
              CUSTOM_COLOR_GRID_VERTICAL_INSET,
            ),
          }}
        />
      </div>

      <div
        aria-label="Choose custom avatar color hue"
        aria-valuemax={360}
        aria-valuemin={0}
        aria-valuenow={hue}
        className="buzz-avatar-hue-scrubber relative mt-3 h-10 w-full cursor-pointer select-none rounded-full touch-none"
        data-testid={`${testIdPrefix}-custom-color-hue`}
        onKeyDown={(event) => {
          if (event.key === "ArrowLeft" || event.key === "ArrowDown") {
            event.preventDefault();
            adjustHue(-6);
          } else if (event.key === "ArrowRight" || event.key === "ArrowUp") {
            event.preventDefault();
            adjustHue(6);
          } else if (event.key === "Home") {
            event.preventDefault();
            onHueChange(0);
          } else if (event.key === "End") {
            event.preventDefault();
            onHueChange(360);
          }
        }}
        onPointerDown={(event) => {
          event.preventDefault();
          lockHueDragSelection();
          event.currentTarget.setPointerCapture(event.pointerId);
          updateHueFromPointer(event);
        }}
        onPointerMove={(event) => {
          if (event.buttons === 1) {
            event.preventDefault();
            updateHueFromPointer(event);
          }
        }}
        onPointerCancel={unlockHueDragSelection}
        onPointerUp={unlockHueDragSelection}
        onLostPointerCapture={unlockHueDragSelection}
        role="slider"
        tabIndex={visible ? 0 : -1}
      >
        <div
          aria-hidden="true"
          className="absolute top-1 h-8 w-8 -translate-x-1/2 rounded-full"
          data-testid={`${testIdPrefix}-custom-color-hue-thumb`}
          style={{
            left: hueScrubberPosition((hue / 360) * 100),
          }}
        >
          <div className="h-full w-full rounded-full bg-white shadow-[0_5px_18px_rgba(0,0,0,0.24),inset_0_0_0_1px_rgba(0,0,0,0.06)]" />
        </div>
      </div>

      <Button
        className="mt-3 h-12 w-full rounded-xl"
        data-testid={`${testIdPrefix}-custom-color-done`}
        onClick={onCommit}
        tabIndex={visible ? 0 : -1}
        type="button"
      >
        Use color
      </Button>
    </motion.div>
  );
}
