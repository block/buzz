import { useReducedMotion } from "motion/react";
import * as React from "react";

import { AnimatedTextSwap } from "@/shared/ui/AnimatedTextSwap";

const SEARCH_PROMPT_WORDS = [
  "everything",
  "a channel",
  "a message",
  "a thread",
  "an agent",
] as const;
const SEARCH_PROMPT_ROTATION_MS = 3200;

export function SearchPromptPlaceholder() {
  const shouldReduceMotion = useReducedMotion();
  const [wordIndex, setWordIndex] = React.useState(0);
  const activeWord = SEARCH_PROMPT_WORDS[wordIndex];

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
      <AnimatedTextSwap
        ariaHidden
        characterTestId="search-placeholder-character"
        value={activeWord}
      />
    </span>
  );
}
