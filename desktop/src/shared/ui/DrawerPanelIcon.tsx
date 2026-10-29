import { motion, useReducedMotion } from "motion/react";

import { cn } from "@/shared/lib/cn";

export function DrawerPanelIcon({
  className,
  side,
  testId,
}: {
  className?: string;
  side: "left" | "right";
  testId?: string;
}) {
  const prefersReducedMotion = useReducedMotion();
  const isOpen = side === "left";

  return (
    <svg
      aria-hidden="true"
      className={cn("h-4 w-auto shrink-0", className)}
      data-testid={testId}
      fill="none"
      height="22"
      viewBox="0 0 24 22"
      width="24"
      xmlns="http://www.w3.org/2000/svg"
    >
      <rect
        height="20"
        rx="5"
        stroke="currentColor"
        strokeWidth="2"
        width="22"
        x="1"
        y="1"
      />
      <motion.rect
        animate={{
          rx: isOpen ? 2.5 : 1,
          width: isOpen ? 5 : 2,
        }}
        fill="currentColor"
        height="14"
        initial={false}
        transition={{
          duration: prefersReducedMotion ? 0 : 0.2,
          ease: "linear",
        }}
        x="4"
        y="4"
      />
    </svg>
  );
}
