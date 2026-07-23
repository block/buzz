import * as React from "react";
import * as CheckboxPrimitive from "@radix-ui/react-checkbox";
import { motion, useReducedMotion } from "motion/react";

import { cn } from "@/shared/lib/cn";

const Checkbox = React.forwardRef<
  React.ElementRef<typeof CheckboxPrimitive.Root>,
  React.ComponentPropsWithoutRef<typeof CheckboxPrimitive.Root>
>(({ className, ...props }, ref) => {
  const shouldReduceMotion = useReducedMotion();

  return (
    <CheckboxPrimitive.Root
      ref={ref}
      className={cn(
        "peer h-4 w-4 shrink-0 rounded-xs border border-primary ring-offset-background focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50 data-[state=checked]:bg-primary data-[state=checked]:text-primary-foreground",
        className,
      )}
      {...props}
    >
      <CheckboxPrimitive.Indicator className="flex items-center justify-center text-current">
        <svg
          aria-hidden="true"
          className="h-4 w-4"
          fill="none"
          viewBox="0 0 24 24"
        >
          <motion.path
            animate={{ opacity: 1, pathLength: 1 }}
            d="m5 12 4 4L19 6"
            initial={shouldReduceMotion ? false : { opacity: 0, pathLength: 0 }}
            stroke="currentColor"
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth="2"
            transition={
              shouldReduceMotion
                ? { duration: 0 }
                : {
                    duration: 0.18,
                    ease: [0.23, 1, 0.32, 1],
                  }
            }
          />
        </svg>
      </CheckboxPrimitive.Indicator>
    </CheckboxPrimitive.Root>
  );
});
Checkbox.displayName = CheckboxPrimitive.Root.displayName;

export { Checkbox };
