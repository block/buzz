import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import * as React from "react";

const SEARCH_PROMPT_WORDS = [
  "everything",
  "a channel",
  "a message",
  "a thread",
  "an agent",
] as const;
const SEARCH_PROMPT_ROTATION_MS = 3200;
const SEARCH_PROMPT_EASE = [0.22, 1, 0.36, 1] as const;
const SEARCH_PROMPT_EXIT_EASE = [0.64, 0, 0.78, 0] as const;
const SEARCH_PROMPT_ENTER_DURATION_SECONDS = 0.54;
const SEARCH_PROMPT_EXIT_DURATION_SECONDS = 0.32;
const SEARCH_PROMPT_ENTER_STAGGER_SECONDS = 0.014;
const SEARCH_PROMPT_EXIT_STAGGER_SECONDS = 0.008;
const SEARCH_PROMPT_Y_OFFSET = "0.5rem";
const SEARCH_PROMPT_NEGATIVE_Y_OFFSET = "-0.5rem";
const SEARCH_PROMPT_BLUR = "0.25rem";

const searchPromptPhraseVariants = {
  animate: {
    transition: {
      staggerChildren: SEARCH_PROMPT_ENTER_STAGGER_SECONDS,
    },
  },
  exit: {
    transition: {
      staggerChildren: SEARCH_PROMPT_EXIT_STAGGER_SECONDS,
    },
  },
  initial: {},
};

const searchPromptCharacterVariants = {
  animate: {
    filter: "blur(0)",
    opacity: 1,
    transition: {
      duration: SEARCH_PROMPT_ENTER_DURATION_SECONDS,
      ease: SEARCH_PROMPT_EASE,
    },
    y: 0,
  },
  exit: {
    filter: `blur(${SEARCH_PROMPT_BLUR})`,
    opacity: 0,
    transition: {
      duration: SEARCH_PROMPT_EXIT_DURATION_SECONDS,
      ease: SEARCH_PROMPT_EXIT_EASE,
    },
    y: SEARCH_PROMPT_NEGATIVE_Y_OFFSET,
  },
  initial: {
    filter: `blur(${SEARCH_PROMPT_BLUR})`,
    opacity: 0,
    y: SEARCH_PROMPT_Y_OFFSET,
  },
};

function getPromptCharacters(value: string) {
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

function getPromptEnterTotalSeconds(characterCount: number) {
  return (
    SEARCH_PROMPT_ENTER_DURATION_SECONDS +
    Math.max(0, characterCount - 1) * SEARCH_PROMPT_ENTER_STAGGER_SECONDS
  );
}

export function SearchPromptPlaceholder() {
  const shouldReduceMotion = useReducedMotion();
  const [wordIndex, setWordIndex] = React.useState(0);
  const activeWord = SEARCH_PROMPT_WORDS[wordIndex];
  const activeCharacters = React.useMemo(
    () => getPromptCharacters(activeWord),
    [activeWord],
  );
  const widthAnimationDurationSeconds = getPromptEnterTotalSeconds(
    activeCharacters.length,
  );
  const measureRef = React.useRef<HTMLSpanElement>(null);
  const pendingWordWidthRef = React.useRef<number | null>(null);
  const [wordWidth, setWordWidth] = React.useState<number | null>(null);

  React.useEffect(() => {
    if (shouldReduceMotion) {
      setWordIndex(0);
      return;
    }

    const intervalId = window.setInterval(() => {
      setWordIndex((currentIndex) => {
        return (currentIndex + 1) % SEARCH_PROMPT_WORDS.length;
      });
    }, SEARCH_PROMPT_ROTATION_MS);

    return () => window.clearInterval(intervalId);
  }, [shouldReduceMotion]);

  React.useLayoutEffect(() => {
    if (shouldReduceMotion || activeWord.length === 0) {
      return;
    }

    const width = measureRef.current?.getBoundingClientRect().width;
    if (typeof width === "number" && Number.isFinite(width)) {
      if (wordWidth === null) {
        setWordWidth(width);
      } else {
        pendingWordWidthRef.current = width;
      }
    }
  }, [activeWord, shouldReduceMotion, wordWidth]);

  const handleWordExitComplete = React.useCallback(() => {
    const nextWidth = pendingWordWidthRef.current;
    if (nextWidth === null) {
      return;
    }

    pendingWordWidthRef.current = null;
    setWordWidth(nextWidth);
  }, []);

  if (shouldReduceMotion) {
    return (
      <span
        aria-hidden="true"
        className="text-muted-foreground"
        data-testid="search-placeholder"
      >
        Search for {activeWord}
      </span>
    );
  }

  return (
    <span
      aria-hidden="true"
      className="pointer-events-none inline-flex min-w-0 items-baseline text-muted-foreground"
      data-active-search-prompt={activeWord}
      data-search-prompt-options={SEARCH_PROMPT_WORDS.join(",")}
      data-testid="search-placeholder"
    >
      <span>Search for&nbsp;</span>
      <span
        className="relative inline-block overflow-visible whitespace-nowrap align-baseline leading-[inherit] motion-safe:transition-[width] motion-safe:ease-[cubic-bezier(0.22,1,0.36,1)] motion-reduce:transition-none"
        data-width-animation-duration-ms={Math.round(
          widthAnimationDurationSeconds * 1000,
        )}
        style={{
          transitionDuration: `${widthAnimationDurationSeconds}s`,
          ...(wordWidth === null ? {} : { width: wordWidth }),
        }}
      >
        <span className="sr-only">everything</span>
        <span
          aria-hidden="true"
          className="pointer-events-none invisible inline-block whitespace-nowrap leading-[inherit]"
          ref={measureRef}
        >
          {activeWord}
        </span>
        <AnimatePresence
          initial={false}
          mode="wait"
          onExitComplete={handleWordExitComplete}
        >
          <motion.span
            aria-hidden="true"
            animate="animate"
            className="absolute inset-x-0 top-0 inline-block whitespace-nowrap leading-[inherit] [transform-style:preserve-3d]"
            exit="exit"
            initial="initial"
            key={activeWord}
            variants={searchPromptPhraseVariants}
          >
            {activeCharacters.map(({ character, key }) => (
              <motion.span
                className="inline-block whitespace-pre [backface-visibility:hidden] [transform-origin:50%_55%] will-change-[transform,opacity,filter]"
                data-testid="search-placeholder-character"
                key={`${activeWord}-${key}`}
                variants={searchPromptCharacterVariants}
              >
                {character}
              </motion.span>
            ))}
          </motion.span>
        </AnimatePresence>
      </span>
    </span>
  );
}
