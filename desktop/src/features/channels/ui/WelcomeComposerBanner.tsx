import * as React from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { Bot, Check } from "lucide-react";

import { cn } from "@/shared/lib/cn";

const WELCOME_PERSONA_NAMES = ["Fizz"] as const;
export const WELCOME_PERSONA_ROTATION_MS = 3200;
export const WELCOME_PERSONA_EASE = [0.22, 1, 0.36, 1] as const;
const WELCOME_PERSONA_EXIT_EASE = [0.64, 0, 0.78, 0] as const;
export const WELCOME_PERSONA_ENTER_DURATION_SECONDS = 0.648;
const WELCOME_PERSONA_EXIT_DURATION_SECONDS = 0.432;
const WELCOME_PERSONA_ENTER_STAGGER_SECONDS = 0.018;
const WELCOME_PERSONA_EXIT_STAGGER_SECONDS = 0.011;
const WELCOME_PERSONA_Y_OFFSET_PX = 9;
const WELCOME_PERSONA_BLUR_PX = 6;
const WELCOME_COMPOSER_BANNER_CONTENT_EXIT_DURATION_SECONDS = 0.18;
const WELCOME_COMPOSER_BANNER_CONTENT_Y_OFFSET_PX = 4;
const WELCOME_COMPOSER_BANNER_SUCCESS_ENTER_DURATION_SECONDS = 0.32;
const WELCOME_COMPOSER_BANNER_SUCCESS_COPY_DELAY_SECONDS = 0.06;
const WELCOME_COMPOSER_BANNER_SUCCESS_Y_OFFSET_PX = 6;
export const WELCOME_COMPOSER_BANNER_DISMISS_DURATION_SECONDS = 0.25;
const WELCOME_COMPOSER_BANNER_DISMISS_BLUR_PX = 3;
export const WELCOME_COMPOSER_BANNER_HIDE_BUFFER_MS = 50;
const WELCOME_COMPOSER_BANNER_DISMISS_Y_OFFSET_PX = 48;
export const WELCOME_COMPOSER_BANNER_SUCCESS_SETTLE_MS = Math.round(
  (WELCOME_COMPOSER_BANNER_CONTENT_EXIT_DURATION_SECONDS +
    WELCOME_COMPOSER_BANNER_SUCCESS_COPY_DELAY_SECONDS +
    WELCOME_COMPOSER_BANNER_SUCCESS_ENTER_DURATION_SECONDS) *
    1000,
);

export type WelcomeComposerBannerState =
  | "prompt"
  | "complete"
  | "dismissing"
  | "hidden";

const welcomePersonaPhraseVariants = {
  animate: {
    transition: {
      staggerChildren: WELCOME_PERSONA_ENTER_STAGGER_SECONDS,
    },
  },
  exit: {
    transition: {
      staggerChildren: WELCOME_PERSONA_EXIT_STAGGER_SECONDS,
    },
  },
  initial: {},
};

const welcomePersonaCharacterVariants = {
  animate: {
    filter: "blur(0px)",
    opacity: 1,
    transition: {
      duration: WELCOME_PERSONA_ENTER_DURATION_SECONDS,
      ease: WELCOME_PERSONA_EASE,
    },
    y: 0,
  },
  exit: {
    filter: `blur(${WELCOME_PERSONA_BLUR_PX}px)`,
    opacity: 0,
    transition: {
      duration: WELCOME_PERSONA_EXIT_DURATION_SECONDS,
      ease: WELCOME_PERSONA_EXIT_EASE,
    },
    y: -WELCOME_PERSONA_Y_OFFSET_PX,
  },
  initial: {
    filter: `blur(${WELCOME_PERSONA_BLUR_PX}px)`,
    opacity: 0,
    y: WELCOME_PERSONA_Y_OFFSET_PX,
  },
};

const welcomeComposerBannerContentVariants = {
  animate: {
    opacity: 1,
    transition: {
      duration: WELCOME_COMPOSER_BANNER_CONTENT_EXIT_DURATION_SECONDS,
      ease: WELCOME_PERSONA_EASE,
    },
    y: 0,
  },
  exit: {
    opacity: 0,
    transition: {
      duration: WELCOME_COMPOSER_BANNER_CONTENT_EXIT_DURATION_SECONDS,
      ease: WELCOME_PERSONA_EXIT_EASE,
    },
    y: -WELCOME_COMPOSER_BANNER_CONTENT_Y_OFFSET_PX,
  },
  initial: {
    opacity: 0,
    y: WELCOME_COMPOSER_BANNER_CONTENT_Y_OFFSET_PX,
  },
};

const welcomeComposerBannerSuccessIconVariants = {
  animate: {
    opacity: 1,
    rotate: 0,
    scale: 1,
    transition: {
      duration: WELCOME_COMPOSER_BANNER_SUCCESS_ENTER_DURATION_SECONDS,
      ease: WELCOME_PERSONA_EASE,
    },
    y: 0,
  },
  initial: {
    opacity: 0,
    rotate: -8,
    scale: 0.72,
    y: WELCOME_COMPOSER_BANNER_SUCCESS_Y_OFFSET_PX,
  },
};

const welcomeComposerBannerSuccessCopyVariants = {
  animate: {
    opacity: 1,
    transition: {
      delay: WELCOME_COMPOSER_BANNER_SUCCESS_COPY_DELAY_SECONDS,
      duration: WELCOME_COMPOSER_BANNER_SUCCESS_ENTER_DURATION_SECONDS,
      ease: WELCOME_PERSONA_EASE,
    },
    y: 0,
  },
  initial: {
    opacity: 0,
    y: WELCOME_COMPOSER_BANNER_SUCCESS_Y_OFFSET_PX,
  },
};

export function containsWelcomePersonaMention(content: string) {
  const normalizedContent = content.toLowerCase();

  return WELCOME_PERSONA_NAMES.some((personaName) =>
    normalizedContent.includes(`@${personaName.toLowerCase()}`),
  );
}

function getWelcomeMentionCharacters(mention: string) {
  const characterCounts = new Map<string, number>();

  return [...mention].map((character) => {
    const occurrence = characterCounts.get(character) ?? 0;
    characterCounts.set(character, occurrence + 1);

    return {
      character,
      key: `${character}-${occurrence}`,
    };
  });
}

function getWelcomePersonaEnterTotalSeconds(characterCount: number) {
  return (
    WELCOME_PERSONA_ENTER_DURATION_SECONDS +
    Math.max(0, characterCount - 1) * WELCOME_PERSONA_ENTER_STAGGER_SECONDS
  );
}

function WelcomeComposerPersonaMention() {
  const shouldReduceMotion = useReducedMotion();
  const [personaIndex, setPersonaIndex] = React.useState(0);
  const activePersonaName = WELCOME_PERSONA_NAMES[personaIndex];
  const activeMention = `@${activePersonaName}`;
  const activeMentionCharacters = React.useMemo(
    () => getWelcomeMentionCharacters(activeMention),
    [activeMention],
  );
  const widthAnimationDurationSeconds = getWelcomePersonaEnterTotalSeconds(
    activeMentionCharacters.length,
  );
  const measureRef = React.useRef<HTMLSpanElement>(null);
  const pendingMentionWidthRef = React.useRef<number | null>(null);
  const [mentionWidth, setMentionWidth] = React.useState<number | null>(null);

  React.useEffect(() => {
    if (shouldReduceMotion) {
      setPersonaIndex(0);
      return;
    }

    const intervalId = window.setInterval(() => {
      setPersonaIndex(
        (currentIndex) => (currentIndex + 1) % WELCOME_PERSONA_NAMES.length,
      );
    }, WELCOME_PERSONA_ROTATION_MS);

    return () => window.clearInterval(intervalId);
  }, [shouldReduceMotion]);

  React.useLayoutEffect(() => {
    if (shouldReduceMotion || activeMention.length === 0) {
      return;
    }

    const width = measureRef.current?.getBoundingClientRect().width;
    if (typeof width === "number" && Number.isFinite(width)) {
      if (mentionWidth === null) {
        setMentionWidth(width);
      } else {
        pendingMentionWidthRef.current = width;
      }
    }
  }, [activeMention, mentionWidth, shouldReduceMotion]);

  const handlePersonaExitComplete = React.useCallback(() => {
    const nextWidth = pendingMentionWidthRef.current;
    if (nextWidth === null) {
      return;
    }

    pendingMentionWidthRef.current = null;
    setMentionWidth(nextWidth);
  }, []);

  if (shouldReduceMotion) {
    return (
      <span
        className="font-medium text-foreground"
        data-animation-target="per-character"
        data-active-persona={activePersonaName}
        data-persona-options={WELCOME_PERSONA_NAMES.join(",")}
        data-testid="welcome-composer-persona-mention"
      >
        {activeMention}
      </span>
    );
  }

  return (
    <span
      className="relative inline-block overflow-visible whitespace-nowrap align-baseline font-medium leading-[inherit] text-foreground motion-safe:transition-[width] motion-safe:ease-[cubic-bezier(0.22,1,0.36,1)] motion-reduce:transition-none"
      data-animation-target="per-character"
      data-active-persona={activePersonaName}
      data-persona-options={WELCOME_PERSONA_NAMES.join(",")}
      data-testid="welcome-composer-persona-mention"
      data-width-animation-duration-ms={Math.round(
        widthAnimationDurationSeconds * 1000,
      )}
      style={{
        transitionDuration: `${widthAnimationDurationSeconds}s`,
        ...(mentionWidth === null ? {} : { width: mentionWidth }),
      }}
    >
      <span className="sr-only">@Fizz</span>
      <span
        aria-hidden
        className="pointer-events-none invisible inline-block whitespace-nowrap leading-[inherit]"
        ref={measureRef}
      >
        {activeMention}
      </span>
      <AnimatePresence
        initial={false}
        mode="wait"
        onExitComplete={handlePersonaExitComplete}
      >
        <motion.span
          aria-hidden
          animate="animate"
          className="absolute inset-x-0 top-0 inline-block whitespace-nowrap leading-[inherit] [transform-style:preserve-3d]"
          exit="exit"
          initial="initial"
          key={activeMention}
          variants={welcomePersonaPhraseVariants}
        >
          {activeMentionCharacters.map(({ character, key }) => (
            <motion.span
              className="inline-block whitespace-pre [backface-visibility:hidden] [transform-origin:50%_55%] will-change-[transform,opacity,filter]"
              data-testid="welcome-composer-persona-character"
              key={`${activeMention}-${key}`}
              variants={welcomePersonaCharacterVariants}
            >
              {character}
            </motion.span>
          ))}
        </motion.span>
      </AnimatePresence>
    </span>
  );
}

type WelcomeComposerBannerProps = {
  state: WelcomeComposerBannerState;
};

export function WelcomeComposerBanner({ state }: WelcomeComposerBannerProps) {
  if (state === "hidden") {
    return null;
  }

  return (
    <AnimatePresence initial={false}>
      <motion.div
        animate={{
          height: state === "dismissing" ? 0 : "auto",
        }}
        className="overflow-visible"
        initial={false}
        transition={{
          duration:
            state === "dismissing"
              ? WELCOME_COMPOSER_BANNER_DISMISS_DURATION_SECONDS
              : WELCOME_PERSONA_ENTER_DURATION_SECONDS,
          ease: WELCOME_PERSONA_EASE,
        }}
      >
        <motion.div
          animate={{
            filter:
              state === "dismissing"
                ? `blur(${WELCOME_COMPOSER_BANNER_DISMISS_BLUR_PX}px)`
                : "blur(0px)",
            opacity: 1,
            y:
              state === "dismissing"
                ? WELCOME_COMPOSER_BANNER_DISMISS_Y_OFFSET_PX
                : 0,
          }}
          className={cn(
            "relative z-0 mx-5 -mb-3 flex transform-gpu items-center gap-2 rounded-t-2xl border border-b-0 px-4 pb-5 pt-2.5 text-sm leading-5 backdrop-blur-sm transition-colors will-change-[filter,transform]",
            state !== "prompt"
              ? "border-emerald-500/30 bg-emerald-500/15 text-foreground"
              : "border-border/60 bg-muted/55 text-muted-foreground",
          )}
          data-state={state}
          data-testid="welcome-composer-guide-banner"
          data-tone={state !== "prompt" ? "success" : "neutral"}
          initial={false}
          transition={{
            duration:
              state === "dismissing"
                ? WELCOME_COMPOSER_BANNER_DISMISS_DURATION_SECONDS
                : WELCOME_PERSONA_ENTER_DURATION_SECONDS,
            ease: WELCOME_PERSONA_EASE,
          }}
        >
          <AnimatePresence initial={false} mode="wait">
            {state !== "prompt" ? (
              <motion.span
                animate="animate"
                className="flex h-4 w-4 shrink-0 items-center justify-center text-foreground"
                data-animation-target="success-icon"
                initial="initial"
                key="complete-icon"
                variants={welcomeComposerBannerSuccessIconVariants}
              >
                <Check
                  aria-hidden
                  className="h-4 w-4"
                  data-testid="welcome-composer-complete-icon"
                />
              </motion.span>
            ) : (
              <motion.span
                animate="animate"
                className="flex h-4 w-4 shrink-0 items-center justify-center text-muted-foreground"
                exit="exit"
                initial="initial"
                key="prompt-icon"
                variants={welcomeComposerBannerContentVariants}
              >
                <Bot aria-hidden className="h-4 w-4" />
              </motion.span>
            )}
          </AnimatePresence>
          <AnimatePresence initial={false} mode="wait">
            {state !== "prompt" ? (
              <motion.span
                animate="animate"
                className="min-w-0"
                data-animation-target="success-copy"
                initial="initial"
                key="complete-copy"
                variants={welcomeComposerBannerSuccessCopyVariants}
              >
                Nice work.
              </motion.span>
            ) : (
              <motion.span
                animate="animate"
                className="min-w-0"
                exit="exit"
                initial="initial"
                key="prompt-copy"
                variants={welcomeComposerBannerContentVariants}
              >
                Try mentioning <WelcomeComposerPersonaMention /> to chat with an
                agent in this channel.
              </motion.span>
            )}
          </AnimatePresence>
        </motion.div>
      </motion.div>
    </AnimatePresence>
  );
}
