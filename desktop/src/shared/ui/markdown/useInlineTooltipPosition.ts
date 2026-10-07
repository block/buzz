import * as React from "react";

const VIEWPORT_INSET = 8;

function distanceToRect(rect: DOMRect, x: number, y: number): number {
  const dx = Math.max(rect.left - x, 0, x - rect.right);
  const dy = Math.max(rect.top - y, 0, y - rect.bottom);
  return dx * dx + dy * dy;
}

/** Keeps a tooltip centered over the inline fragment currently under the pointer. */
export function useInlineTooltipPosition() {
  const contentRef = React.useRef<HTMLDivElement | null>(null);
  const fragmentCenterXRef = React.useRef<number | null>(null);
  const placementObserverRef = React.useRef<MutationObserver | null>(null);
  const placedRef = React.useRef(false);

  const positionContent = React.useCallback(() => {
    const content = contentRef.current;
    const fragmentCenterX = fragmentCenterXRef.current;
    if (!content || fragmentCenterX === null || !placedRef.current) return;
    const wrapper = content.parentElement;
    if (!wrapper) return;
    const rect = content.getBoundingClientRect();
    const maxLeft = Math.max(
      VIEWPORT_INSET,
      window.innerWidth - VIEWPORT_INSET - rect.width,
    );
    const desiredLeft = Math.min(
      Math.max(fragmentCenterX - rect.width / 2, VIEWPORT_INSET),
      maxLeft,
    );
    const correction = desiredLeft - rect.left;
    if (Math.abs(correction) < 0.5) return;
    const currentOffset = Number.parseFloat(wrapper.style.marginLeft) || 0;
    wrapper.style.marginLeft = `${currentOffset + correction}px`;
  }, []);

  const setContentRef = React.useCallback(
    (content: HTMLDivElement | null) => {
      placementObserverRef.current?.disconnect();
      placementObserverRef.current = null;
      contentRef.current = content;
      placedRef.current = false;
      const wrapper = content?.parentElement;
      if (!content || !wrapper) return;

      let transform = wrapper.style.transform;
      const handlePlacement = () => {
        const nextTransform = wrapper.style.transform;
        if (nextTransform === transform) return;
        transform = nextTransform;
        placedRef.current = true;
        positionContent();
      };
      const observer = new MutationObserver(handlePlacement);
      observer.observe(wrapper, {
        attributeFilter: ["style"],
        attributes: true,
      });
      placementObserverRef.current = observer;
      requestAnimationFrame(() => {
        if (contentRef.current !== content) return;
        placedRef.current = true;
        positionContent();
      });
    },
    [positionContent],
  );

  React.useEffect(() => () => placementObserverRef.current?.disconnect(), []);

  const onPointerMove = React.useCallback(
    (event: React.PointerEvent<HTMLElement>) => {
      const rects = Array.from(event.currentTarget.getClientRects()).filter(
        (rect) => rect.width > 0 && rect.height > 0,
      );
      if (rects.length <= 1) {
        fragmentCenterXRef.current = null;
        contentRef.current?.parentElement?.style.removeProperty("margin-left");
        return;
      }
      const fragment = rects.reduce<DOMRect | null>((nearest, rect) => {
        if (!nearest) return rect;
        return distanceToRect(rect, event.clientX, event.clientY) <
          distanceToRect(nearest, event.clientX, event.clientY)
          ? rect
          : nearest;
      }, null);
      if (!fragment) return;
      fragmentCenterXRef.current = fragment.left + fragment.width / 2;
      positionContent();
    },
    [positionContent],
  );

  return {
    contentRef: setContentRef,
    onPointerMove,
  };
}
