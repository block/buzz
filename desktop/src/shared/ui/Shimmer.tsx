import type { CSSProperties } from "react";

import { cn } from "@/shared/lib/cn";

type ShimmerProps = {
  children: string;
  className?: string;
};

export function Shimmer({ children, className }: ShimmerProps) {
  return (
    <span
      className={cn("buzz-shimmer", className)}
      style={
        { "--buzz-shimmer-spread": `${children.length * 2}px` } as CSSProperties
      }
    >
      {children}
    </span>
  );
}
