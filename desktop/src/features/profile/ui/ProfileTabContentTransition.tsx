import * as React from "react";
import {
  AnimatePresence,
  motion,
  type Variants,
  useReducedMotion,
} from "motion/react";

import type { ProfilePanelTab } from "@/features/profile/ui/UserProfilePanelUtils";
import { cn } from "@/shared/lib/cn";

type TabTransitionDirection = -1 | 0 | 1;

type TabTransitionContext = {
  direction: TabTransitionDirection;
  reduceMotion: boolean;
};

const TAB_CONTENT_OFFSET_PX = 28;

const tabContentVariants: Variants = {
  enter: ({ direction, reduceMotion }: TabTransitionContext) => ({
    opacity: reduceMotion ? 1 : 0.72,
    pointerEvents: "auto",
    transform: reduceMotion
      ? "translateX(0px)"
      : `translateX(${direction * TAB_CONTENT_OFFSET_PX}px)`,
  }),
  center: {
    opacity: 1,
    pointerEvents: "auto",
    transform: "translateX(0px)",
  },
  exit: ({ direction, reduceMotion }: TabTransitionContext) => ({
    opacity: reduceMotion ? 1 : 0,
    pointerEvents: "none",
    transform: reduceMotion
      ? "translateX(0px)"
      : `translateX(${-direction * TAB_CONTENT_OFFSET_PX}px)`,
  }),
};

export function ProfileTabContentTransition({
  activeTab,
  children,
  className,
  tabs,
}: {
  activeTab: ProfilePanelTab;
  children: React.ReactNode;
  className?: string;
  tabs: ProfilePanelTab[];
}) {
  const reduceMotion = useReducedMotion() ?? false;
  const [lastTransition, setLastTransition] = React.useState<{
    direction: TabTransitionDirection;
    tab: ProfilePanelTab;
  }>({ direction: 0, tab: activeTab });
  const previousIndex = tabs.indexOf(lastTransition.tab);
  const activeIndex = tabs.indexOf(activeTab);
  const direction: TabTransitionDirection =
    lastTransition.tab === activeTab
      ? lastTransition.direction
      : previousIndex < 0 || activeIndex < 0 || previousIndex === activeIndex
        ? 0
        : activeIndex > previousIndex
          ? 1
          : -1;
  const transitionContext: TabTransitionContext = {
    direction,
    reduceMotion,
  };

  React.useLayoutEffect(() => {
    if (lastTransition.tab !== activeTab) {
      setLastTransition({ direction, tab: activeTab });
    }
  }, [activeTab, direction, lastTransition.tab]);

  return (
    <div
      className={cn("relative overflow-x-clip", className)}
      data-active-tab={activeTab}
      data-testid="user-profile-tab-content-transition"
      data-transition-direction={
        direction === 0 ? "none" : direction > 0 ? "forward" : "backward"
      }
    >
      <AnimatePresence
        custom={transitionContext}
        initial={false}
        mode="popLayout"
      >
        <motion.div
          animate="center"
          className="w-full min-w-0"
          custom={transitionContext}
          exit="exit"
          initial="enter"
          key={activeTab}
          transition={
            reduceMotion
              ? { duration: 0 }
              : {
                  duration: 0.22,
                  ease: [0.23, 1, 0.32, 1],
                }
          }
          variants={tabContentVariants}
        >
          {children}
        </motion.div>
      </AnimatePresence>
    </div>
  );
}
