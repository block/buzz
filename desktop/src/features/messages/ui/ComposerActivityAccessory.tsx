import type { ReactNode } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";

import { cn } from "@/shared/lib/cn";

type ComposerActivityAccessoryProps = {
  children: ReactNode;
  className?: string;
  testId?: string;
  visible: boolean;
};

/**
 * Fades activity content into the composer dock's reserved bottom rail without
 * changing layout height. This keeps timeline scroll padding stable.
 */
export function ComposerActivityAccessory({
  children,
  className,
  testId,
  visible,
}: ComposerActivityAccessoryProps) {
  const prefersReducedMotion = useReducedMotion();

  return (
    <AnimatePresence initial={false}>
      {visible ? (
        <motion.div
          animate={{ opacity: 1 }}
          className={cn(
            "composer-dock-activity absolute inset-x-0 z-10 bg-background",
            className,
          )}
          data-testid={testId}
          exit={{ opacity: 0 }}
          initial={{ opacity: 0 }}
          transition={{
            duration: prefersReducedMotion ? 0 : 0.2,
            ease: [0.23, 1, 0.32, 1],
          }}
        >
          {children}
        </motion.div>
      ) : null}
    </AnimatePresence>
  );
}
