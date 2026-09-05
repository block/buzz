import * as React from "react";

import { cn } from "@/shared/lib/cn";

type SegmentOption<Value extends string> = {
  value: Value;
  label: string;
  Icon?: React.ComponentType<{ className?: string }>;
};

type SegmentedControlSize = "compact" | "default" | "wide";
type SegmentedControlAppearance = "default" | "onboarding-inline";

const SIZE_CLASSES: Record<SegmentedControlSize, string> = {
  compact: "w-48",
  default: "w-60",
  wide: "w-72",
};

/** A mutually exclusive control with equal-width, optionally scrubbable options. */
export function SegmentedControl<Value extends string>({
  appearance = "default",
  className,
  disabled = false,
  indicatorTestId,
  legend,
  onPreviewChange,
  onValueChange,
  optionTestIdPrefix,
  options,
  size = "default",
  testId,
  value,
}: {
  appearance?: SegmentedControlAppearance;
  className?: string;
  disabled?: boolean;
  indicatorTestId?: string;
  legend: string;
  onPreviewChange?: (value: Value | null) => void;
  onValueChange: (value: Value) => void;
  optionTestIdPrefix: string;
  options: readonly SegmentOption<Value>[];
  size?: SegmentedControlSize;
  testId: string;
  value: Value;
}) {
  const [previewValue, setPreviewValue] = React.useState<Value | null>(null);
  const controlRef = React.useRef<HTMLFieldSetElement | null>(null);
  const optionRefs = React.useRef<Array<HTMLButtonElement | null>>([]);
  const [inlineIndicator, setInlineIndicator] = React.useState({
    left: 3,
    width: 0,
  });
  const activePointerIdRef = React.useRef<number | null>(null);
  const pointerStartXRef = React.useRef<number | null>(null);
  const pointerStartValueRef = React.useRef<Value | null>(null);
  const scrubValueRef = React.useRef<Value | null>(null);
  const skipPointerClickRef = React.useRef(false);
  const displayedValue = previewValue ?? value;
  const isOnboardingInline = appearance === "onboarding-inline";
  const selectedIndex = Math.max(
    0,
    options.findIndex((option) => option.value === displayedValue),
  );

  React.useLayoutEffect(() => {
    if (!isOnboardingInline) return;
    const control = controlRef.current;
    const selectedOption = optionRefs.current[selectedIndex];
    if (!control || !selectedOption) return;

    const updateIndicator = () => {
      const controlBounds = control.getBoundingClientRect();
      const optionBounds = selectedOption.getBoundingClientRect();
      setInlineIndicator({
        left: optionBounds.left - controlBounds.left,
        width: optionBounds.width,
      });
    };

    updateIndicator();
    const observer = new ResizeObserver(updateIndicator);
    observer.observe(control);
    observer.observe(selectedOption);
    return () => observer.disconnect();
  }, [isOnboardingInline, selectedIndex]);

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

  const cancelScrub = React.useCallback(() => {
    const control = controlRef.current;
    const pointerId = activePointerIdRef.current;
    activePointerIdRef.current = null;
    if (control && pointerId != null && control.hasPointerCapture(pointerId)) {
      control.releasePointerCapture(pointerId);
    }
    pointerStartXRef.current = null;
    pointerStartValueRef.current = null;
    skipPointerClickRef.current = false;
    preview(null);
  }, [preview]);

  React.useEffect(() => {
    const handleWindowBlur = () => cancelScrub();
    globalThis.addEventListener?.("blur", handleWindowBlur);
    return () => {
      globalThis.removeEventListener?.("blur", handleWindowBlur);
      cancelScrub();
    };
  }, [cancelScrub]);

  const handlePointerDown = (
    event: React.PointerEvent<HTMLFieldSetElement>,
  ) => {
    if (!onPreviewChange || event.button !== 0) return;
    event.currentTarget.setPointerCapture(event.pointerId);
    activePointerIdRef.current = event.pointerId;
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
    activePointerIdRef.current = null;
    event.currentTarget.releasePointerCapture(event.pointerId);
    pointerStartXRef.current = null;
    pointerStartValueRef.current = null;
    onValueChange(nextValue);
    preview(null);
    globalThis.setTimeout(() => {
      skipPointerClickRef.current = false;
    }, 0);
  };

  const handlePointerCancel = () => cancelScrub();

  const handleLostPointerCapture = () => {
    if (activePointerIdRef.current != null) cancelScrub();
  };

  return (
    <fieldset
      className={cn(
        "relative isolate max-w-full shrink-0 overflow-hidden",
        isOnboardingInline
          ? "h-9 rounded-full border border-[#d4d4d4] bg-[#e2e2e2]/30 p-[3px] text-[#0f0f0f]"
          : "h-8 rounded-md bg-muted/45 p-0.5",
        SIZE_CLASSES[size],
        onPreviewChange && "touch-none select-none cursor-ew-resize",
        "disabled:pointer-events-none disabled:opacity-50",
        className,
      )}
      data-slot="segmented-control"
      data-testid={testId}
      disabled={disabled}
      onLostPointerCapture={handleLostPointerCapture}
      onPointerCancel={handlePointerCancel}
      onPointerDown={handlePointerDown}
      onPointerMove={handlePointerMove}
      onPointerUp={handlePointerUp}
      ref={controlRef}
    >
      <legend className="sr-only">{legend}</legend>
      <div
        aria-hidden="true"
        className={cn(
          "absolute z-0 transition-[transform,width] duration-200 ease-in-out motion-reduce:transition-none",
          isOnboardingInline
            ? "bottom-[3px] left-0 top-[3px] rounded-full bg-[#e2e2e2]"
            : "bottom-0.5 left-0.5 top-0.5 rounded-md bg-background shadow-sm",
          previewValue && "duration-0",
        )}
        data-testid={indicatorTestId ?? `${testId}-indicator`}
        style={{
          transform: isOnboardingInline
            ? `translateX(${inlineIndicator.left}px)`
            : `translateX(${selectedIndex * 100}%)`,
          width: isOnboardingInline
            ? inlineIndicator.width
            : `calc((100% - 0.25rem) / ${options.length})`,
        }}
      />
      {/* Legends escape grid/flex layout on a fieldset, so the columns live
          on an inner wrapper the legend is not part of. */}
      <div
        className={cn(
          "h-full",
          isOnboardingInline
            ? "flex w-fit gap-0.5"
            : "grid auto-cols-fr grid-flow-col",
        )}
      >
        {options.map(({ value: optionValue, label, Icon }, optionIndex) => (
          <button
            aria-pressed={value === optionValue}
            className={cn(
              "relative z-10 flex h-full items-center justify-center gap-1.5 bg-transparent font-medium transition-colors duration-150 ease-out focus-visible:outline-hidden motion-reduce:transition-none",
              isOnboardingInline
                ? "rounded-full px-4 py-1 text-sm text-[#0f0f0f] focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-[#0f0f0f]"
                : "rounded-md px-2.5 text-xs focus-visible:ring-2 focus-visible:ring-ring",
              !isOnboardingInline &&
                (displayedValue === optionValue
                  ? "text-foreground"
                  : "text-muted-foreground hover:text-foreground"),
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
            ref={(element) => {
              optionRefs.current[optionIndex] = element;
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
