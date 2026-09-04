import type * as React from "react";

type PortalledScrollAreaProps = React.ComponentPropsWithoutRef<"div">;

/**
 * Keeps a portalled overflow container wheel-scrollable when it is rendered
 * outside a modal's scroll-lock boundary.
 */
export function PortalledScrollArea({
  onWheel,
  ...props
}: PortalledScrollAreaProps) {
  function handleWheel(event: React.WheelEvent<HTMLDivElement>) {
    onWheel?.(event);
    if (event.defaultPrevented) return;

    const area = event.currentTarget;
    const maxScrollTop = area.scrollHeight - area.clientHeight;
    if (maxScrollTop <= 0) return;

    const multiplier =
      event.deltaMode === WheelEvent.DOM_DELTA_LINE
        ? 16
        : event.deltaMode === WheelEvent.DOM_DELTA_PAGE
          ? area.clientHeight
          : 1;
    const nextScrollTop = Math.max(
      0,
      Math.min(maxScrollTop, area.scrollTop + event.deltaY * multiplier),
    );
    if (nextScrollTop === area.scrollTop) return;

    area.scrollTop = nextScrollTop;
    event.preventDefault();
    event.stopPropagation();
  }

  return <div {...props} onWheel={handleWheel} />;
}
