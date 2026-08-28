import { motion, useReducedMotion } from "motion/react";
import * as React from "react";

import { cn } from "@/shared/lib/cn";

export const MESSAGE_ACTION_BLOOM_EASE_OUT = [0.23, 1, 0.32, 1] as const;
export const MESSAGE_ACTION_BLOOM_SPEED_MULTIPLIER = 1.8;
export const MESSAGE_ACTION_BLOOM_VISUAL_DURATION =
  0.15 / MESSAGE_ACTION_BLOOM_SPEED_MULTIPLIER;

const MESSAGE_ACTION_RESTING_RADIUS = 32;
const MESSAGE_ACTION_EXPANDED_RADIUS = 24;

type MessageActionBloomSurfaceProps = Omit<
  React.ComponentPropsWithoutRef<typeof motion.div>,
  "children"
> & {
  children: React.ReactNode;
  expanded: boolean;
  size?: { height: number; width: number } | null;
};

/** A persistent, measurement-driven surface that never scales its children. */
export const MessageActionBloomSurface = React.forwardRef<
  HTMLDivElement,
  MessageActionBloomSurfaceProps
>(function MessageActionBloomSurface(
  { children, className, expanded, size, style, ...props },
  ref,
) {
  const reduceMotion = useReducedMotion();

  return (
    <motion.div
      animate={{
        borderRadius: expanded
          ? MESSAGE_ACTION_EXPANDED_RADIUS
          : MESSAGE_ACTION_RESTING_RADIUS,
        ...(size ? { height: size.height, width: size.width } : {}),
      }}
      className={cn(
        "contain-layout contain-paint overflow-hidden border border-border/70 bg-background/95 shadow-xs backdrop-blur-sm supports-[backdrop-filter]:bg-background/85",
        className,
      )}
      initial={false}
      ref={ref}
      style={{
        ...style,
        transformOrigin: "bottom right",
        willChange: "width, height, border-radius",
      }}
      transition={
        reduceMotion
          ? { duration: 0 }
          : {
              borderRadius: {
                bounce: 0,
                type: "spring",
                visualDuration: MESSAGE_ACTION_BLOOM_VISUAL_DURATION,
              },
              height: {
                bounce: 0,
                type: "spring",
                visualDuration: MESSAGE_ACTION_BLOOM_VISUAL_DURATION,
              },
              width: {
                bounce: 0,
                type: "spring",
                visualDuration: MESSAGE_ACTION_BLOOM_VISUAL_DURATION,
              },
            }
      }
      {...props}
    >
      {children}
    </motion.div>
  );
});
