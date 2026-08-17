import type { ReactNode } from "react";
import { cn } from "@/shared/lib/cn";

export function GradientLayer() {
  return (
    <div
      aria-hidden="true"
      className="buzz-theme-gradient-layer pointer-events-none absolute inset-0 -z-10"
      data-buzz-gradient-layer
    >
      <div className="buzz-theme-gradient-underlay absolute inset-0" />
      <div
        className="buzz-theme-gradient-layer-light absolute inset-0 opacity-0"
        data-buzz-gradient="light"
      />
      <div
        className="buzz-theme-gradient-layer-dark absolute inset-0 opacity-0"
        data-buzz-gradient="dark"
      />
    </div>
  );
}

export function ContentSurface({
  children,
  insetLeft = false,
  unframed = false,
  terminal,
}: {
  children: ReactNode;
  /** Match the outer card gutter when no sidebar is visible to its left. */
  insetLeft?: boolean;
  terminal?: ReactNode;
  /** Used by dedicated huddle windows, which should not resemble app cards. */
  unframed?: boolean;
}) {
  return (
    <div
      className={cn(
        "relative z-10 flex min-h-0 flex-1 flex-col overflow-hidden bg-background",
        unframed
          ? undefined
          : "mb-2 mr-2 mt-px rounded-2xl shadow-content-edge motion-safe:transition-[margin] motion-safe:duration-200 motion-safe:ease-linear",
        !unframed && (insetLeft ? "ml-2" : "ml-px"),
      )}
      data-buzz-content-surface
      data-buzz-content-unframed={unframed ? true : undefined}
    >
      <div className="buzz-content-primary flex min-h-0 flex-1 flex-col overflow-hidden">
        {children}
      </div>
      <div className="buzz-terminal-dock-host" data-terminal-dock>
        {terminal}
      </div>
    </div>
  );
}
