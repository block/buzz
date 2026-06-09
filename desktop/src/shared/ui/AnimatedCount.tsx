import * as React from "react";

import { cn } from "@/shared/lib/cn";
import {
  formatAnimatedCount,
  getAnimatedCountSlots,
  normalizeAnimatedCount,
} from "@/shared/ui/animatedCountParts";

type CountDirection = -1 | 0 | 1;

type AnimatedCountTransition = {
  current: string;
  direction: CountDirection;
  previous: string;
  version: number;
};

type AnimatedCountProps = {
  className?: string;
  value: number;
};

const ANIMATED_COUNT_SETTLE_DELAY_MS = 260;

function directionFor(previous: number, current: number): CountDirection {
  if (current > previous) return 1;
  if (current < previous) return -1;
  return 0;
}

function renderSlotText(value: string) {
  return value || "\u00A0";
}

function AnimatedCountSlot({
  current,
  direction,
  isDigit,
  previous,
  version,
}: {
  current: string;
  direction: CountDirection;
  isDigit: boolean;
  previous: string;
  version: number;
}) {
  if (direction === 0 || previous === current) {
    return (
      <span
        className={cn(
          "sprout-animated-count__slot",
          isDigit && "sprout-animated-count__slot--digit",
        )}
      >
        {renderSlotText(current)}
      </span>
    );
  }

  if (!isDigit) {
    return (
      <span
        className="sprout-animated-count__symbol"
        data-direction={direction > 0 ? "up" : "down"}
        key={`${version}-${previous}-${current}`}
      >
        {renderSlotText(current)}
      </span>
    );
  }

  return (
    <span
      className="sprout-animated-count__slot sprout-animated-count__slot--digit"
      data-direction={direction > 0 ? "up" : "down"}
    >
      <span
        className="sprout-animated-count__reel"
        key={`${version}-${previous}-${current}`}
      >
        {direction > 0 ? (
          <>
            <span>{renderSlotText(previous)}</span>
            <span>{renderSlotText(current)}</span>
          </>
        ) : (
          <>
            <span>{renderSlotText(current)}</span>
            <span>{renderSlotText(previous)}</span>
          </>
        )}
      </span>
    </span>
  );
}

export function AnimatedCount({ className, value }: AnimatedCountProps) {
  const normalizedValue = normalizeAnimatedCount(value);
  const formattedValue = formatAnimatedCount(normalizedValue);
  const previousValueRef = React.useRef(normalizedValue);
  const [transition, setTransition] = React.useState<AnimatedCountTransition>(
    () => ({
      current: formattedValue,
      direction: 0,
      previous: formattedValue,
      version: 0,
    }),
  );

  React.useLayoutEffect(() => {
    setTransition((currentTransition) => {
      if (currentTransition.current === formattedValue) {
        previousValueRef.current = normalizedValue;
        return currentTransition;
      }

      const direction = directionFor(previousValueRef.current, normalizedValue);
      previousValueRef.current = normalizedValue;

      return {
        current: formattedValue,
        direction,
        previous: currentTransition.current,
        version: currentTransition.version + 1,
      };
    });
  }, [formattedValue, normalizedValue]);

  React.useEffect(() => {
    if (
      transition.direction === 0 ||
      transition.previous === transition.current
    ) {
      return;
    }

    const timeoutId = window.setTimeout(() => {
      setTransition((currentTransition) => {
        if (currentTransition.version !== transition.version) {
          return currentTransition;
        }

        return {
          ...currentTransition,
          direction: 0,
          previous: currentTransition.current,
        };
      });
    }, ANIMATED_COUNT_SETTLE_DELAY_MS);

    return () => window.clearTimeout(timeoutId);
  }, [
    transition.current,
    transition.direction,
    transition.previous,
    transition.version,
  ]);

  const slots = getAnimatedCountSlots(transition.previous, transition.current);

  return (
    <span className={cn("sprout-animated-count", className)}>
      <span className="sr-only">{transition.current}</span>
      <span aria-hidden className="sprout-animated-count__motion">
        {slots.map((slot) => (
          <AnimatedCountSlot
            current={slot.current}
            direction={transition.direction}
            isDigit={slot.isDigit}
            key={`${transition.version}-${slot.place}`}
            previous={slot.previous}
            version={transition.version}
          />
        ))}
      </span>
      <span aria-hidden className="sprout-animated-count__static">
        {transition.current}
      </span>
    </span>
  );
}
