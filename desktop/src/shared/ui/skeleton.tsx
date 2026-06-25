import * as React from "react";

import { cn } from "@/shared/lib/cn";

type SkeletonProps = React.HTMLAttributes<HTMLDivElement> & {
  pulsing?: boolean;
};

function Skeleton({ className, pulsing = true, ...props }: SkeletonProps) {
  return (
    <div
      aria-hidden="true"
      className={cn(
        "t-skel-bar rounded-md bg-primary/10",
        pulsing && "is-pulsing",
        className,
      )}
      {...props}
    />
  );
}

type SkeletonRevealProps = React.HTMLAttributes<HTMLDivElement> & {
  contentClassName?: string;
  layout?: "absolute" | "flow";
  loading: boolean;
  skeleton: React.ReactNode;
  skeletonClassName?: string;
};

function SkeletonReveal({
  children,
  className,
  contentClassName,
  layout = "flow",
  loading,
  skeleton,
  skeletonClassName,
  ...props
}: SkeletonRevealProps) {
  const rootRef = React.useRef<HTMLDivElement>(null);
  const previousLoadingRef = React.useRef(loading);
  const [isResetting, setIsResetting] = React.useState(false);

  React.useLayoutEffect(() => {
    const wasLoading = previousLoadingRef.current;
    previousLoadingRef.current = loading;

    if (!loading || wasLoading) return;

    setIsResetting(true);
    rootRef.current?.getBoundingClientRect();

    const reset = () => setIsResetting(false);
    const frameId = globalThis.requestAnimationFrame
      ? globalThis.requestAnimationFrame(reset)
      : globalThis.setTimeout(reset, 0);

    return () => {
      if (typeof frameId === "number") {
        if (globalThis.cancelAnimationFrame) {
          globalThis.cancelAnimationFrame(frameId);
        } else {
          globalThis.clearTimeout(frameId);
        }
      }
    };
  }, [loading]);

  return (
    <div
      className={cn(
        "t-skel",
        !loading && "is-revealed",
        isResetting && "is-resetting",
        className,
      )}
      data-layout={layout}
      data-state={loading ? "loading" : "loaded"}
      ref={rootRef}
      {...props}
    >
      <div
        aria-hidden="true"
        className={cn("t-skel-skeleton is-pulsing", skeletonClassName)}
      >
        {skeleton}
      </div>
      <div
        aria-hidden={loading}
        className={cn("t-skel-content", contentClassName)}
      >
        {children}
      </div>
    </div>
  );
}

export { Skeleton, SkeletonReveal };
