import * as React from "react";

export function useTranscriptBubbleOverflow(enabled: boolean) {
  const ref = React.useRef<HTMLDivElement>(null);
  const [hasOverflow, setHasOverflow] = React.useState(false);

  React.useLayoutEffect(() => {
    const element = ref.current;
    if (!enabled || !element) {
      setHasOverflow(false);
      return;
    }

    const updateOverflow = () => {
      setHasOverflow(element.scrollHeight > element.clientHeight + 1);
    };

    updateOverflow();

    if (typeof ResizeObserver === "undefined") {
      return;
    }

    const observer = new ResizeObserver(updateOverflow);
    observer.observe(element);
    if (element.firstElementChild) {
      observer.observe(element.firstElementChild);
    }

    return () => observer.disconnect();
  }, [enabled]);

  return [ref, hasOverflow] as const;
}
