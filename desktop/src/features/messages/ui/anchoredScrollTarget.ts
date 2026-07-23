const TOP_ALIGNED_TARGET_INSET_PX = 12;

export type ScrollTargetAlignment = "bottom" | "center" | "top-with-divider";

export function makeTargetKey(
  id: string | null,
  alignment: ScrollTargetAlignment,
) {
  return id ? `${alignment}:${id}` : null;
}

export function resolveTargetDivider({
  alignment,
  container,
  firstMessageId,
  targetMessageId,
}: {
  alignment: ScrollTargetAlignment;
  container: HTMLElement;
  firstMessageId: string | undefined;
  targetMessageId: string;
}): HTMLElement | null | undefined {
  if (alignment !== "top-with-divider") return null;
  const divider = container.querySelector<HTMLElement>(
    '[data-testid="message-unread-divider"]',
  );
  // The row can commit one render before its unread divider. Keep a non-first
  // target unresolved so a later DOM retry aligns the marker, not the row.
  // The first reply intentionally has no divider and may use the row.
  if (firstMessageId !== targetMessageId && !divider) return undefined;
  return divider;
}

export function getTargetScrollPlacement({
  alignment,
  containerClientHeight,
  containerScrollHeight,
  containerScrollTop,
  containerTop,
  dividerTop,
  targetHeight,
  targetTop,
}: {
  alignment: ScrollTargetAlignment;
  containerClientHeight: number;
  containerScrollHeight: number;
  containerScrollTop: number;
  containerTop: number;
  dividerTop: number | null;
  targetHeight: number;
  targetTop: number;
}) {
  const currentTopOffset = targetTop - containerTop;
  const maxScrollTop = Math.max(
    0,
    containerScrollHeight - containerClientHeight,
  );
  let targetScrollTop: number;
  if (alignment === "bottom") {
    targetScrollTop = maxScrollTop;
  } else if (alignment === "top-with-divider") {
    targetScrollTop = Math.min(
      maxScrollTop,
      Math.max(
        0,
        containerScrollTop +
          ((dividerTop ?? targetTop) - containerTop) -
          TOP_ALIGNED_TARGET_INSET_PX,
      ),
    );
  } else {
    const centeredTopOffset = (containerClientHeight - targetHeight) / 2;
    targetScrollTop = Math.min(
      maxScrollTop,
      Math.max(0, containerScrollTop + currentTopOffset - centeredTopOffset),
    );
  }

  return {
    contentTop: targetTop + containerScrollTop - containerTop,
    maxScrollTop,
    targetScrollTop,
    targetTopOffset: currentTopOffset - (targetScrollTop - containerScrollTop),
  };
}
