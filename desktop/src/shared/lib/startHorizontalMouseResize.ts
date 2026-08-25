import type * as React from "react";

type HorizontalMouseResizeOptions = {
  indicatorText?: string;
  onFinish?: () => void;
  onStart?: () => void;
};

export function formatHorizontalResizeIndicator(
  widthPx: number,
  containerWidthPx: number,
): string {
  const percentage =
    containerWidthPx > 0 ? Math.round((widthPx / containerWidthPx) * 100) : 0;
  return `${Math.round(widthPx)} px · ${percentage}%`;
}

/** Keep a desktop resize drag inside the host document, including over iframes. */
export function startHorizontalMouseResize(
  event: React.MouseEvent<HTMLButtonElement>,
  onMove: (clientX: number) => string | undefined,
  options: HorizontalMouseResizeOptions = {},
): void {
  event.preventDefault();

  const previousCursor = document.body.style.cursor;
  const previousUserSelect = document.body.style.userSelect;
  const dragShield = document.createElement("div");
  dragShield.setAttribute("aria-hidden", "true");
  Object.assign(dragShield.style, {
    cursor: "col-resize",
    inset: "0",
    position: "fixed",
    userSelect: "none",
    zIndex: "2147483647",
  });
  const indicator = document.createElement("div");
  indicator.dataset.testid = "horizontal-resize-indicator";
  indicator.className =
    "pointer-events-none fixed rounded-md border border-border/70 bg-popover/95 px-2 py-1 font-mono text-2xs font-semibold tabular-nums text-popover-foreground shadow-md backdrop-blur-sm";
  indicator.style.top = `${Math.max(52, Math.min(event.clientY, window.innerHeight - 52))}px`;
  indicator.style.transform = "translateY(-50%)";
  indicator.textContent = options.indicatorText ?? "";
  dragShield.append(indicator);

  const positionIndicator = (clientX: number) => {
    indicator.style.left = `${Math.max(12, Math.min(clientX + 14, window.innerWidth - 116))}px`;
  };
  positionIndicator(event.clientX);
  document.body.append(dragShield);
  document.body.style.cursor = "col-resize";
  document.body.style.userSelect = "none";
  options.onStart?.();

  const handleMouseMove = (moveEvent: MouseEvent) => {
    moveEvent.preventDefault();
    const nextText = onMove(moveEvent.clientX);
    if (nextText != null) indicator.textContent = nextText;
    positionIndicator(moveEvent.clientX);
  };

  const finishResize = () => {
    document.body.style.cursor = previousCursor;
    document.body.style.userSelect = previousUserSelect;
    dragShield.remove();
    window.removeEventListener("mousemove", handleMouseMove);
    window.removeEventListener("mouseup", finishResize);
    window.removeEventListener("blur", finishResize);
    options.onFinish?.();
  };

  window.addEventListener("mousemove", handleMouseMove);
  window.addEventListener("mouseup", finishResize);
  window.addEventListener("blur", finishResize);
}
