import type * as React from "react";
import { Slot } from "@radix-ui/react-slot";

import { cn } from "@/shared/lib/cn";

type BubbleProps = React.ComponentProps<"div"> & {
  asChild?: boolean;
  side?: "left" | "right";
  variant?: "default" | "muted" | "outline" | "ghost";
};

function Bubble({
  asChild,
  className,
  side = "left",
  variant = "default",
  ...props
}: BubbleProps) {
  const Comp = asChild ? Slot : "div";

  return (
    <Comp
      className={cn(
        "min-w-0 rounded-lg px-3.5 py-2.5 text-base leading-6 shadow-xs",
        variant === "default" && side === "left" && "bg-muted text-foreground",
        variant === "default" &&
          side === "right" &&
          "bg-primary text-primary-foreground",
        variant === "muted" && "bg-muted/60 text-foreground",
        variant === "outline" &&
          "border border-border/70 bg-background text-foreground",
        variant === "ghost" && "bg-transparent px-0 shadow-none",
        className,
      )}
      data-side={side}
      data-slot="bubble"
      data-variant={variant}
      {...props}
    />
  );
}

export { Bubble };
