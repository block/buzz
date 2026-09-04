import { motion, useReducedMotion } from "motion/react";
import type { ComponentProps } from "react";

import { cn } from "@/shared/lib/cn";

export function TerminalPanelIcon({
  className,
  open,
  ...props
}: ComponentProps<"svg"> & {
  open: boolean;
}) {
  const prefersReducedMotion = useReducedMotion();

  return (
    <svg
      aria-hidden="true"
      className={cn("h-4 w-auto shrink-0", className)}
      fill="none"
      height="22"
      viewBox="0 0 24 22"
      width="24"
      xmlns="http://www.w3.org/2000/svg"
      {...props}
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
          height: open ? 5 : 2,
          rx: open ? 2.5 : 1,
          y: open ? 13 : 16,
        }}
        fill="currentColor"
        initial={false}
        transition={{
          duration: prefersReducedMotion ? 0 : 0.2,
          ease: "easeOut",
        }}
        width="16"
        x="4"
      />
    </svg>
  );
}
