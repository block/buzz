import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import * as React from "react";

import { cn } from "@/shared/lib/cn";

const ANIMATED_TEXT_SWAP_EASE = [0.22, 1, 0.36, 1] as const;
const ANIMATED_TEXT_SWAP_EXIT_EASE = [0.64, 0, 0.78, 0] as const;
const ANIMATED_TEXT_SWAP_ENTER_DURATION_SECONDS = 0.54;
const ANIMATED_TEXT_SWAP_EXIT_DURATION_SECONDS = 0.32;
const ANIMATED_TEXT_SWAP_ENTER_STAGGER_SECONDS = 0.014;
const ANIMATED_TEXT_SWAP_EXIT_STAGGER_SECONDS = 0.008;
const ANIMATED_TEXT_SWAP_Y_OFFSET = "0.5rem";
const ANIMATED_TEXT_SWAP_NEGATIVE_Y_OFFSET = "-0.5rem";
const ANIMATED_TEXT_SWAP_BLUR = "0.25rem";

const animatedTextSwapPhraseVariants = {
  animate: {
    transition: {
      staggerChildren: ANIMATED_TEXT_SWAP_ENTER_STAGGER_SECONDS,
    },
  },
  exit: {
    transition: {
      staggerChildren: ANIMATED_TEXT_SWAP_EXIT_STAGGER_SECONDS,
    },
  },
  initial: {},
};

const animatedTextSwapCharacterVariants = {
  animate: {
    filter: "blur(0)",
    opacity: 1,
    transition: {
      duration: ANIMATED_TEXT_SWAP_ENTER_DURATION_SECONDS,
      ease: ANIMATED_TEXT_SWAP_EASE,
    },
    y: 0,
  },
  exit: {
    filter: `blur(${ANIMATED_TEXT_SWAP_BLUR})`,
    opacity: 0,
    transition: {
      duration: ANIMATED_TEXT_SWAP_EXIT_DURATION_SECONDS,
      ease: ANIMATED_TEXT_SWAP_EXIT_EASE,
    },
    y: ANIMATED_TEXT_SWAP_NEGATIVE_Y_OFFSET,
  },
  initial: {
    filter: `blur(${ANIMATED_TEXT_SWAP_BLUR})`,
    opacity: 0,
    y: ANIMATED_TEXT_SWAP_Y_OFFSET,
  },
};

function getAnimatedTextCharacters(value: string) {
  const characterCounts = new Map<string, number>();

  return [...value].map((character) => {
    const occurrence = characterCounts.get(character) ?? 0;
    characterCounts.set(character, occurrence + 1);

    return {
      character,
      key: `${character}-${occurrence}`,
    };
  });
}

function getAnimatedTextEnterTotalSeconds(characterCount: number) {
  return (
    ANIMATED_TEXT_SWAP_ENTER_DURATION_SECONDS +
    Math.max(0, characterCount - 1) * ANIMATED_TEXT_SWAP_ENTER_STAGGER_SECONDS
  );
}

type AnimatedTextSwapProps = {
  ariaHidden?: boolean;
  characterTestId?: string;
  className?: string;
  textClassName?: string;
  value: string;
};

export function AnimatedTextSwap({
  ariaHidden = false,
  characterTestId,
  className,
  textClassName,
  value,
}: AnimatedTextSwapProps) {
  const shouldReduceMotion = useReducedMotion();
  const activeCharacters = React.useMemo(
    () => getAnimatedTextCharacters(value),
    [value],
  );
  const widthAnimationDurationSeconds = getAnimatedTextEnterTotalSeconds(
    activeCharacters.length,
  );
  const measureRef = React.useRef<HTMLSpanElement>(null);
  const pendingTextWidthRef = React.useRef<number | null>(null);
  const [textWidth, setTextWidth] = React.useState<number | null>(null);

  React.useLayoutEffect(() => {
    if (shouldReduceMotion || value.length === 0) {
      return;
    }

    const width = measureRef.current?.getBoundingClientRect().width;
    if (typeof width === "number" && Number.isFinite(width)) {
      if (textWidth === null) {
        setTextWidth(width);
      } else {
        pendingTextWidthRef.current = width;
      }
    }
  }, [shouldReduceMotion, textWidth, value]);

  const handleTextExitComplete = React.useCallback(() => {
    const nextWidth = pendingTextWidthRef.current;
    if (nextWidth === null) {
      return;
    }

    pendingTextWidthRef.current = null;
    setTextWidth(nextWidth);
  }, []);

  if (shouldReduceMotion) {
    return (
      <span aria-hidden={ariaHidden || undefined} className={className}>
        {value}
      </span>
    );
  }

  return (
    <span
      aria-hidden={ariaHidden || undefined}
      className={cn(
        "relative inline-block overflow-visible whitespace-nowrap align-baseline leading-[inherit] motion-safe:transition-[width] motion-safe:ease-[cubic-bezier(0.22,1,0.36,1)] motion-reduce:transition-none",
        className,
      )}
      data-width-animation-duration-ms={Math.round(
        widthAnimationDurationSeconds * 1000,
      )}
      style={{
        transitionDuration: `${widthAnimationDurationSeconds}s`,
        ...(textWidth === null ? {} : { width: textWidth }),
      }}
    >
      {ariaHidden ? null : <span className="sr-only">{value}</span>}
      <span
        aria-hidden="true"
        className="pointer-events-none invisible inline-block whitespace-nowrap leading-[inherit]"
        ref={measureRef}
      >
        {value}
      </span>
      <AnimatePresence
        initial={false}
        mode="wait"
        onExitComplete={handleTextExitComplete}
      >
        <motion.span
          aria-hidden="true"
          animate="animate"
          className={cn(
            "absolute inset-x-0 top-0 inline-block whitespace-nowrap leading-[inherit] [transform-style:preserve-3d]",
            textClassName,
          )}
          exit="exit"
          initial="initial"
          key={value}
          variants={animatedTextSwapPhraseVariants}
        >
          {activeCharacters.map(({ character, key }) => (
            <motion.span
              className="inline-block whitespace-pre [backface-visibility:hidden] [transform-origin:50%_55%] will-change-[transform,opacity,filter]"
              data-testid={characterTestId}
              key={`${value}-${key}`}
              variants={animatedTextSwapCharacterVariants}
            >
              {character}
            </motion.span>
          ))}
        </motion.span>
      </AnimatePresence>
    </span>
  );
}
