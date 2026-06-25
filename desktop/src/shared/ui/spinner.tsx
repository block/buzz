import type * as React from "react";

import { cn } from "@/shared/lib/cn";

type SpinnerProps = React.ComponentPropsWithoutRef<"span"> & {
  className?: string;
  size?: number | string;
};

export function Spinner({
  children,
  className,
  size,
  role = "status",
  "aria-label": ariaLabel = "Loading",
  "aria-hidden": ariaHidden,
  style,
  ...rest
}: SpinnerProps) {
  const isDecorative = ariaHidden === true || ariaHidden === "true";

  return (
    <span
      aria-hidden={ariaHidden}
      className={cn(
        "sprout-arc-spinner inline-block h-6 w-6 shrink-0 rounded-full border-4 border-current/10 border-t-current",
        className,
      )}
      role={isDecorative ? undefined : role}
      style={{
        ...(size === undefined ? null : { height: size, width: size }),
        ...style,
      }}
      {...rest}
    >
      {children}
      {isDecorative ? null : <span className="sr-only">{ariaLabel}</span>}
    </span>
  );
}
