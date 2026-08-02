import type { ReactNode } from "react";

export function GradientLayer() {
  return (
    <div
      aria-hidden="true"
      className="buzz-theme-gradient-layer pointer-events-none absolute inset-0 -z-10"
      data-buzz-gradient-layer
    >
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

export function ContentSurface({ children }: { children: ReactNode }) {
  return (
    <div
      className="relative z-10 mb-1 ml-px mr-1 mt-px flex min-h-0 flex-1 flex-col overflow-hidden rounded-content-surface bg-background shadow-content-edge"
      data-buzz-content-surface
    >
      {children}
    </div>
  );
}
