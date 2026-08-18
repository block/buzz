import * as React from "react";
import type { ReactNode } from "react";
import { X } from "lucide-react";
import {
  AnimatePresence,
  motion,
  type Transition,
  useReducedMotion,
} from "motion/react";

import { cn } from "@/shared/lib/cn";
import {
  POOF_DURATION_MS,
  POOF_ORIGIN_CLASS,
  POOF_TRIGGER_CLASS,
} from "@/shared/ui/PoofBurstProvider";

type SidebarActionCardTone = "neutral" | "success";
export type SidebarActionCardSurface = "background" | "secondary";

type SidebarCompactActionCardProps = {
  actionAriaLabel: string;
  actionDisabled?: boolean;
  actionTestId?: string;
  className?: string;
  description?: string;
  dismissClassName?: string;
  dismissLabel?: string;
  iconKey?: string;
  icon: ReactNode;
  onAction: () => void;
  onDismiss?: () => void;
  role?: "alert" | "status";
  surface?: SidebarActionCardSurface;
  testId: string;
  title: string;
  tone?: SidebarActionCardTone;
};

type SidebarActionDismissButtonProps = {
  className?: string;
  isDismissing: boolean;
  label: string;
  onDismiss: () => void;
  onDismissStart: () => void;
  testId: string;
};

type SidebarActionDescriptionTransition = {
  current: string;
  isAnimating: boolean;
  previous: string;
  version: number;
};

const SIDEBAR_ACTION_DESCRIPTION_SETTLE_DELAY_MS = 260;

function SidebarActionDescriptionText({
  shouldReduceMotion,
  value,
}: {
  shouldReduceMotion: boolean;
  value: string;
}) {
  const [transition, setTransition] =
    React.useState<SidebarActionDescriptionTransition>(() => ({
      current: value,
      isAnimating: false,
      previous: value,
      version: 0,
    }));

  React.useLayoutEffect(() => {
    setTransition((currentTransition) => {
      if (currentTransition.current === value) {
        return currentTransition;
      }

      return {
        current: value,
        isAnimating: !shouldReduceMotion,
        previous: currentTransition.current,
        version: currentTransition.version + 1,
      };
    });
  }, [shouldReduceMotion, value]);

  React.useEffect(() => {
    if (!transition.isAnimating) {
      return;
    }

    const timeoutId = window.setTimeout(() => {
      setTransition((currentTransition) => {
        if (currentTransition.version !== transition.version) {
          return currentTransition;
        }

        return {
          ...currentTransition,
          isAnimating: false,
          previous: currentTransition.current,
        };
      });
    }, SIDEBAR_ACTION_DESCRIPTION_SETTLE_DELAY_MS);

    return () => window.clearTimeout(timeoutId);
  }, [transition.isAnimating, transition.version]);

  if (shouldReduceMotion || !transition.isAnimating) {
    return (
      <span className="buzz-sidebar-action-description">
        {transition.current}
      </span>
    );
  }

  return (
    <span className="buzz-sidebar-action-description">
      <span className="sr-only">{transition.current}</span>
      <span aria-hidden className="buzz-sidebar-action-description__motion">
        <span
          className="buzz-sidebar-action-description__reel"
          key={`${transition.version}-${transition.previous}-${transition.current}`}
        >
          <span>{transition.previous}</span>
          <span>{transition.current}</span>
        </span>
      </span>
    </span>
  );
}

function SidebarActionDismissButton({
  className,
  isDismissing,
  label,
  onDismiss,
  onDismissStart,
  testId,
}: SidebarActionDismissButtonProps) {
  const dismissTimeoutRef = React.useRef<number | null>(null);

  React.useEffect(() => {
    return () => {
      if (dismissTimeoutRef.current !== null) {
        window.clearTimeout(dismissTimeoutRef.current);
      }
    };
  }, []);

  return (
    <button
      aria-label={label}
      className={cn(
        "group/dismiss pointer-events-none absolute -right-1 -top-2 z-10 h-6 w-6 rounded-full text-muted-foreground/45 transition-colors duration-150 ease-out hover:text-foreground/80 focus-visible:pointer-events-auto focus-visible:text-foreground/80 focus-visible:outline-hidden group-hover/sidebar-action-card:pointer-events-auto group-hover/sidebar-compact-action-card:pointer-events-auto",
        POOF_TRIGGER_CLASS,
        className,
      )}
      data-testid={testId}
      disabled={isDismissing}
      onClick={(event) => {
        event.stopPropagation();
        if (dismissTimeoutRef.current !== null) {
          return;
        }
        onDismissStart();
        dismissTimeoutRef.current = window.setTimeout(() => {
          dismissTimeoutRef.current = null;
          onDismiss();
        }, POOF_DURATION_MS);
      }}
      type="button"
    >
      <span className="flex h-full w-full scale-95 items-center justify-center rounded-full bg-background opacity-0 shadow-sm ring-1 ring-border/70 transition-[opacity,transform] duration-150 ease-[cubic-bezier(0.23,1,0.32,1)] motion-reduce:scale-100 group-focus-visible/dismiss:scale-100 group-focus-visible/dismiss:opacity-100 group-focus-visible/dismiss:ring-2 group-focus-visible/dismiss:ring-muted-foreground/40 group-hover/sidebar-action-card:scale-100 group-hover/sidebar-action-card:opacity-100 group-hover/sidebar-compact-action-card:scale-100 group-hover/sidebar-compact-action-card:opacity-100">
        <X aria-hidden="true" className="h-4 w-4" />
      </span>
    </button>
  );
}

function dismissTestId(testId: string) {
  return testId === "sidebar-update-card"
    ? "sidebar-update-dismiss"
    : `${testId}-dismiss`;
}

export function SidebarCompactActionCard({
  actionAriaLabel,
  actionDisabled = false,
  actionTestId,
  className,
  description,
  dismissClassName,
  dismissLabel = "Dismiss notification",
  icon,
  iconKey,
  onAction,
  onDismiss,
  role,
  surface = "background",
  testId,
  title,
  tone = "neutral",
}: SidebarCompactActionCardProps) {
  const shouldReduceMotion = useReducedMotion();
  const isSuccess = tone === "success";
  const [isDismissing, setIsDismissing] = React.useState(false);
  const resolvedIconKey = iconKey ?? title;
  const cardTransition: Transition = shouldReduceMotion
    ? { duration: 0.08 }
    : {
        duration: 0.28,
        ease: [0.22, 1, 0.36, 1] as const,
      };
  const cardHiddenState = shouldReduceMotion
    ? { opacity: 0 }
    : { opacity: 0, scale: 0.9 };
  const cardVisibleState = shouldReduceMotion
    ? { opacity: 1 }
    : { opacity: 1, scale: 1 };
  const contentTransition: Transition = shouldReduceMotion
    ? { duration: 0 }
    : {
        duration: 0.16,
        ease: [0.22, 1, 0.36, 1] as const,
      };
  const contentInitial = shouldReduceMotion
    ? { opacity: 0 }
    : { opacity: 0, y: 3 };
  const contentExit = shouldReduceMotion
    ? { opacity: 0 }
    : { opacity: 0, y: -3 };

  return (
    <motion.div
      animate={isDismissing ? cardHiddenState : cardVisibleState}
      className={cn(
        "group/sidebar-compact-action-card relative w-full origin-bottom",
        POOF_ORIGIN_CLASS,
        isDismissing && "pointer-events-none",
        className,
      )}
      data-dismissing={isDismissing ? "true" : undefined}
      data-testid={testId}
      exit={cardHiddenState}
      initial={cardHiddenState}
      role={role}
      transition={cardTransition}
    >
      <button
        aria-label={actionAriaLabel}
        className={cn(
          "flex w-full items-center gap-3 rounded-xl border px-3 py-3 text-left shadow-xs transition-[background-color,border-color,color,box-shadow] duration-150 ease-out focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-muted-foreground/40 disabled:cursor-default disabled:opacity-100",
          isSuccess
            ? "buzz-sidebar-action-card--success disabled:cursor-default disabled:opacity-100"
            : surface === "secondary"
              ? "border-border/70 bg-secondary/80 text-secondary-foreground hover:border-border hover:bg-secondary dark:bg-secondary/60 dark:hover:bg-secondary/70"
              : "border-border/70 bg-background/70 text-foreground hover:border-border hover:bg-muted/40 dark:bg-background/50 dark:hover:bg-muted/30",
        )}
        data-testid={actionTestId}
        disabled={actionDisabled}
        onClick={onAction}
        type="button"
      >
        <motion.span
          className={cn(
            "relative flex min-h-10 min-w-0 flex-1 flex-col justify-center",
            description && "top-[0.1875rem]",
          )}
          layout="position"
          transition={contentTransition}
        >
          <AnimatePresence initial={false} mode="wait">
            <motion.span
              animate={{ opacity: 1, y: 0 }}
              className="block text-sm font-semibold leading-tight"
              exit={contentExit}
              initial={contentInitial}
              key={title}
              transition={contentTransition}
            >
              {title}
            </motion.span>
          </AnimatePresence>
          <AnimatePresence initial={false}>
            {description ? (
              <motion.span
                animate={{ opacity: 1, y: 0 }}
                className="mt-1 block text-xs leading-snug text-muted-foreground"
                exit={contentExit}
                initial={contentInitial}
                key="description"
                transition={contentTransition}
              >
                <SidebarActionDescriptionText
                  shouldReduceMotion={Boolean(shouldReduceMotion)}
                  value={description}
                />
              </motion.span>
            ) : null}
          </AnimatePresence>
        </motion.span>
        <motion.span
          className={cn(
            "ml-auto flex h-10 w-10 shrink-0 items-center justify-center transition-colors duration-150 ease-out",
            isSuccess
              ? "buzz-sidebar-action-card__success-icon"
              : "text-muted-foreground group-hover/sidebar-compact-action-card:text-foreground",
          )}
          layout="position"
          transition={contentTransition}
        >
          <AnimatePresence initial={false} mode="wait">
            <motion.span
              animate={{ opacity: 1, scale: 1 }}
              className="flex h-5 w-5 items-center justify-center"
              exit={{ opacity: 0, scale: shouldReduceMotion ? 1 : 0.95 }}
              initial={{ opacity: 0, scale: shouldReduceMotion ? 1 : 0.95 }}
              key={resolvedIconKey}
              transition={contentTransition}
            >
              {icon}
            </motion.span>
          </AnimatePresence>
        </motion.span>
      </button>
      {onDismiss ? (
        <SidebarActionDismissButton
          className={dismissClassName}
          isDismissing={isDismissing}
          label={dismissLabel}
          onDismiss={onDismiss}
          onDismissStart={() => setIsDismissing(true)}
          testId={dismissTestId(testId)}
        />
      ) : null}
    </motion.div>
  );
}
