import * as React from "react";

const ENTRANCE_ANIMATION = "motion-enter-send-open";

function prefersReducedMotion() {
  return (
    typeof window !== "undefined" &&
    window.matchMedia?.("(prefers-reduced-motion: reduce)").matches === true
  );
}

/**
 * Opens the user's own just-sent message into the timeline once: the row grows
 * from zero height while it fades in (`.motion-enter-send`), so a
 * bottom-pinned timeline glides up instead of jumping by a whole row.
 *
 * The class is dropped when the grow finishes or is cancelled, because its
 * child clips while growing and would otherwise cut off the row's hover action
 * rail. With reduced motion the class is never applied.
 */
export function SendEntrance({ children }: { children: React.ReactNode }) {
  const [isEntering, setIsEntering] = React.useState(
    () => !prefersReducedMotion(),
  );
  const ref = React.useRef<HTMLDivElement>(null);

  React.useEffect(() => {
    const element = ref.current;
    if (!isEntering || !element) return;
    // React has no animationcancel prop, so listen natively for both outcomes.
    const finish = (event: AnimationEvent) => {
      if (
        event.target === element &&
        event.animationName === ENTRANCE_ANIMATION
      ) {
        setIsEntering(false);
      }
    };
    element.addEventListener("animationend", finish);
    element.addEventListener("animationcancel", finish);
    return () => {
      element.removeEventListener("animationend", finish);
      element.removeEventListener("animationcancel", finish);
    };
  }, [isEntering]);

  return (
    <div
      className={isEntering ? "motion-enter-send" : undefined}
      data-testid="message-send-entrance"
      ref={ref}
    >
      <div>{children}</div>
    </div>
  );
}
