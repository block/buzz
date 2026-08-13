import * as React from "react";

import { cn } from "@/shared/lib/cn";

type SegmentOption<Value extends string> = {
  value: Value;
  label: string;
  Icon?: React.ComponentType<{ className?: string }>;
};

export function SettingsSegmentedControl<Value extends string>({
  className,
  indicatorTestId,
  legend,
  onPreviewChange,
  onValueChange,
  optionTestIdPrefix,
  options,
  testId,
  value,
}: {
  className?: string;
  indicatorTestId?: string;
  legend: string;
  onPreviewChange?: (value: Value | null) => void;
  onValueChange: (value: Value) => void;
  optionTestIdPrefix: string;
  options: readonly SegmentOption<Value>[];
  testId: string;
  value: Value;
}) {
  const [previewValue, setPreviewValue] = React.useState<Value | null>(null);
  const pointerStartXRef = React.useRef<number | null>(null);
  const pointerStartValueRef = React.useRef<Value | null>(null);
  const scrubValueRef = React.useRef<Value | null>(null);
  const skipPointerClickRef = React.useRef(false);
  const displayedValue = previewValue ?? value;
  const selectedIndex = Math.max(
    0,
    options.findIndex((option) => option.value === displayedValue),
  );

  const getValueAtPointer = React.useCallback(
    (element: HTMLFieldSetElement, clientX: number): Value => {
      const bounds = element.getBoundingClientRect();
      const position = Math.max(
        0,
        Math.min(bounds.width - 1, clientX - bounds.left),
      );
      const index = Math.min(
        options.length - 1,
        Math.floor((position / bounds.width) * options.length),
      );
      return options[index]?.value ?? value;
    },
    [options, value],
  );

  const preview = React.useCallback(
    (nextValue: Value | null) => {
      scrubValueRef.current = nextValue;
      setPreviewValue(nextValue);
      onPreviewChange?.(nextValue);
    },
    [onPreviewChange],
  );

  React.useEffect(
    () => () => {
      onPreviewChange?.(null);
    },
    [onPreviewChange],
  );

  const handlePointerDown = (
    event: React.PointerEvent<HTMLFieldSetElement>,
  ) => {
    if (!onPreviewChange || event.button !== 0) return;
    event.currentTarget.setPointerCapture(event.pointerId);
    pointerStartXRef.current = event.clientX;
    pointerStartValueRef.current = getValueAtPointer(
      event.currentTarget,
      event.clientX,
    );
    scrubValueRef.current = null;
    skipPointerClickRef.current = true;
    event.preventDefault();
  };

  const handlePointerMove = (
    event: React.PointerEvent<HTMLFieldSetElement>,
  ) => {
    if (!event.currentTarget.hasPointerCapture(event.pointerId)) return;
    const nextValue = getValueAtPointer(event.currentTarget, event.clientX);
    const pointerStartX = pointerStartXRef.current;
    const pointerStartValue = pointerStartValueRef.current;
    const crossedDragThreshold =
      pointerStartX != null && Math.abs(event.clientX - pointerStartX) >= 4;
    if (
      scrubValueRef.current == null &&
      !crossedDragThreshold &&
      nextValue === pointerStartValue
    ) {
      return;
    }
    if (nextValue !== scrubValueRef.current) preview(nextValue);
  };

  const handlePointerUp = (event: React.PointerEvent<HTMLFieldSetElement>) => {
    if (!event.currentTarget.hasPointerCapture(event.pointerId)) return;
    const nextValue = getValueAtPointer(event.currentTarget, event.clientX);
    event.currentTarget.releasePointerCapture(event.pointerId);
    pointerStartXRef.current = null;
    pointerStartValueRef.current = null;
    onValueChange(nextValue);
    preview(null);
    globalThis.setTimeout(() => {
      skipPointerClickRef.current = false;
    }, 0);
  };

  const handlePointerCancel = (
    event: React.PointerEvent<HTMLFieldSetElement>,
  ) => {
    if (!event.currentTarget.hasPointerCapture(event.pointerId)) return;
    event.currentTarget.releasePointerCapture(event.pointerId);
    pointerStartXRef.current = null;
    pointerStartValueRef.current = null;
    skipPointerClickRef.current = false;
    preview(null);
  };

  return (
    <fieldset
      className={cn(
        "relative isolate h-8 w-60 shrink-0 overflow-hidden rounded-md bg-muted/45 p-0.5",
        onPreviewChange && "touch-none select-none cursor-ew-resize",
        className,
      )}
      data-testid={testId}
      onPointerCancel={handlePointerCancel}
      onPointerDown={handlePointerDown}
      onPointerMove={handlePointerMove}
      onPointerUp={handlePointerUp}
    >
      <legend className="sr-only">{legend}</legend>
      <div
        aria-hidden="true"
        className={cn(
          "absolute bottom-0.5 left-0.5 top-0.5 z-0 rounded-md bg-background shadow-sm transition-transform duration-200 ease-in-out motion-reduce:transition-none",
          previewValue && "duration-0",
        )}
        data-testid={indicatorTestId ?? `${testId}-indicator`}
        style={{
          transform: `translateX(${selectedIndex * 100}%)`,
          width: `calc((100% - 0.25rem) / ${options.length})`,
        }}
      />
      {/* Legends escape grid/flex layout on a fieldset, so the columns live
          on an inner wrapper the legend is not part of. */}
      <div className="grid h-full auto-cols-fr grid-flow-col">
        {options.map(({ value: optionValue, label, Icon }) => (
          <button
            aria-pressed={value === optionValue}
            className={cn(
              "relative z-10 flex h-full items-center justify-center gap-1.5 rounded-md bg-transparent px-2.5 text-xs font-medium transition-colors duration-150 ease-out focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring motion-reduce:transition-none",
              displayedValue === optionValue
                ? "text-foreground"
                : "text-muted-foreground hover:text-foreground",
            )}
            data-testid={`${optionTestIdPrefix}-${optionValue}`}
            key={optionValue}
            onClick={(event) => {
              if (event.detail > 0 && skipPointerClickRef.current) {
                skipPointerClickRef.current = false;
                return;
              }
              onValueChange(optionValue);
            }}
            type="button"
          >
            {Icon ? <Icon className="h-3.5 w-3.5" /> : null}
            {label}
          </button>
        ))}
      </div>
    </fieldset>
  );
}
