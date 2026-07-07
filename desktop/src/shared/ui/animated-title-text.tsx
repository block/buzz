import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import * as React from "react";

import { cn } from "@/shared/lib/cn";

// Mirrors the search placeholder swap (SearchPromptPlaceholder): the old text
// rises out with a blur while the new text rises in, characters staggered,
// and the container width glides between the two measured text widths.
const TITLE_SWAP_EASE = [0.22, 1, 0.36, 1] as const;
const TITLE_SWAP_EXIT_EASE = [0.64, 0, 0.78, 0] as const;
const TITLE_SWAP_ENTER_DURATION_SECONDS = 0.54;
const TITLE_SWAP_EXIT_DURATION_SECONDS = 0.32;
const TITLE_SWAP_ENTER_STAGGER_SECONDS = 0.014;
const TITLE_SWAP_EXIT_STAGGER_SECONDS = 0.008;
const TITLE_SWAP_Y_OFFSET = "0.5rem";
const TITLE_SWAP_NEGATIVE_Y_OFFSET = "-0.5rem";
const TITLE_SWAP_BLUR = "0.25rem";

const titleSwapPhraseVariants = {
  animate: {
    transition: { staggerChildren: TITLE_SWAP_ENTER_STAGGER_SECONDS },
  },
  exit: {
    transition: { staggerChildren: TITLE_SWAP_EXIT_STAGGER_SECONDS },
  },
  initial: {},
};

const titleSwapCharacterVariants = {
  animate: {
    filter: "blur(0)",
    opacity: 1,
    transition: {
      duration: TITLE_SWAP_ENTER_DURATION_SECONDS,
      ease: TITLE_SWAP_EASE,
    },
    y: 0,
  },
  exit: {
    filter: `blur(${TITLE_SWAP_BLUR})`,
    opacity: 0,
    transition: {
      duration: TITLE_SWAP_EXIT_DURATION_SECONDS,
      ease: TITLE_SWAP_EXIT_EASE,
    },
    y: TITLE_SWAP_NEGATIVE_Y_OFFSET,
  },
  initial: {
    filter: `blur(${TITLE_SWAP_BLUR})`,
    opacity: 0,
    y: TITLE_SWAP_Y_OFFSET,
  },
};

function getTitleCharacters(value: string) {
  const characterCounts = new Map<string, number>();
  return [...value].map((character) => {
    const occurrence = characterCounts.get(character) ?? 0;
    characterCounts.set(character, occurrence + 1);
    return { character, key: `${character}-${occurrence}` };
  });
}

/**
 * Text that swaps with the search-placeholder character animation whenever
 * `text` changes. The first render never animates, so mounting (opening a
 * view) shows the text immediately — only in-place renames animate.
 *
 * The wrapper width is measured explicitly from an invisible copy of the
 * text (same approach as SearchPromptPlaceholder). Shrink-to-fit sizing
 * derived from the absolutely positioned animation layer is circular and
 * collapses the box.
 */
export function AnimatedTitleText({
  className,
  text,
}: {
  className?: string;
  text: string;
}) {
  const shouldReduceMotion = useReducedMotion();
  const measureRef = React.useRef<HTMLSpanElement>(null);
  const pendingWidthRef = React.useRef<number | null>(null);
  const [width, setWidth] = React.useState<number | null>(null);

  React.useLayoutEffect(() => {
    if (shouldReduceMotion || text.length === 0) {
      return;
    }
    const measured = measureRef.current?.getBoundingClientRect().width;
    if (typeof measured !== "number" || !Number.isFinite(measured)) {
      return;
    }
    if (width === null) {
      // First measurement: apply immediately (no swap in flight).
      setWidth(measured);
    } else {
      // Hold the new width until the old text finishes exiting so the box
      // resizes together with the incoming text.
      pendingWidthRef.current = measured;
    }
  }, [shouldReduceMotion, text, width]);

  const handleExitComplete = React.useCallback(() => {
    const nextWidth = pendingWidthRef.current;
    if (nextWidth === null) {
      return;
    }
    pendingWidthRef.current = null;
    setWidth(nextWidth);
  }, []);

  if (shouldReduceMotion) {
    return <span className={cn("truncate", className)}>{text}</span>;
  }

  const characters = getTitleCharacters(text);

  return (
    <span
      className={cn(
        "relative inline-block max-w-full overflow-hidden whitespace-nowrap align-baseline leading-[inherit] motion-safe:transition-[width] motion-safe:duration-300 motion-safe:ease-[cubic-bezier(0.22,1,0.36,1)]",
        className,
      )}
      data-testid="animated-title-text"
      style={width === null ? undefined : { width }}
    >
      <span className="sr-only">{text}</span>
      {/* Invisible copy at natural width — the measurement source that keeps
          the wrapper sized while the visible layer animates absolutely. */}
      <span
        aria-hidden="true"
        className="pointer-events-none invisible inline-block whitespace-nowrap leading-[inherit]"
        ref={measureRef}
      >
        {text}
      </span>
      <AnimatePresence
        initial={false}
        mode="wait"
        onExitComplete={handleExitComplete}
      >
        <motion.span
          animate="animate"
          aria-hidden="true"
          className="absolute left-0 top-0 inline-block whitespace-nowrap leading-[inherit] [transform-style:preserve-3d]"
          exit="exit"
          initial="initial"
          key={text}
          variants={titleSwapPhraseVariants}
        >
          {characters.map(({ character, key }) => (
            <motion.span
              className="inline-block whitespace-pre [backface-visibility:hidden] [transform-origin:50%_55%] will-change-[transform,opacity,filter]"
              key={`${text}-${key}`}
              variants={titleSwapCharacterVariants}
            >
              {character}
            </motion.span>
          ))}
        </motion.span>
      </AnimatePresence>
    </span>
  );
}
