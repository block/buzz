import type { CSSProperties, ReactNode } from "react";

/**
 * Adds a moving highlight over always-readable text.
 *
 * The base layer never animates or becomes transparent, so motion is additive:
 * a renderer or timing edge cannot make the status disappear.
 */
export function ShimmerText({ children }: { children: ReactNode }) {
  const text = typeof children === "string" ? children : "";

  return (
    <span
      className="buzz-shimmer-text"
      data-shimmer-text={text}
      style={{ "--shimmer-spread": `${text.length * 2.5}px` } as CSSProperties}
    >
      {children}
    </span>
  );
}
