import * as React from "react";

import { BESTIE_POPOVER_SHORTCUT_EVENT } from "@/shared/lib/keyboard-shortcuts";
import { cn } from "@/shared/lib/cn";
import {
  acquireNativeWebviewModalPark,
  releaseNativeWebviewModalPark,
} from "@/shared/lib/nativeWebviewModalPark";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { BestiePopover, BestieTriggerVisual } from "./BestiePopover";
import {
  setBestiePopoverOpen,
  useBestieHasUnreadMessage,
} from "./bestieAttentionStore";
import { useBestieNudge } from "./bestieNudgeStore";
import {
  setBestiePopoverListsCollapsed,
  useBestiePopoverListsCollapsed,
} from "./bestiePopoverListsPreference";
import {
  BESTIE_POPOVER_MAX_MAX_HEIGHT_PX,
  BESTIE_POPOVER_MAX_WIDTH_PX,
  BESTIE_POPOVER_MIN_WIDTH_PX,
  bestiePopoverMinHeightPx,
  bestiePopoverViewportMaxHeightPx,
  setBestiePopoverSize,
  useBestiePopoverSize,
} from "./bestiePopoverSizePreference";
import { useBestie } from "./useBestie";

/**
 * Anchored Bestie trigger for the sidebar profile footer.
 * Shows agent avatar at bottom; name appears beside it when assigned.
 * Left-nav label is "Assistant". Opens the Assistant popover chat.
 * No agent: still opens Assistant (empty); + inside the popover goes to Agents.
 * Phase 2: proactive wake nudge shows a distinct badge (not a DM unread).
 * Phase 3: due-reminder nudges auto-open the popover with the nudge banner.
 * Unread agent replies (popover + Bestie DM closed) show a pulsing light ring.
 * Ring clears on open/view and never pulses for stale hydrate / system noise.
 *
 * Popover is always-on-top of the React chrome (high z-index) so it stacks
 * above the app UI. Pinned / playground WKWebViews stay mounted and visible
 * underneath while idle — do not park on open. During edge drag we briefly
 * park natives and drive size from window pointermove/up (handle-only move
 * is eaten once the pointer crosses WKWebViews). Width handle stays on the
 * popover's right edge so the initial grab is on the React surface, not the
 * native window edge zone.
 * Drag-resizable (width + height) with persisted size and min/max clamps.
 * Height is explicit (empty chat space OK). Min height keeps the message
 * area visible; when Lists are open, min also includes Lists/reminders.
 */
export function BestieProfileTrigger({ className }: { className?: string }) {
  const bestie = useBestie();
  const [open, setOpen] = React.useState(false);
  const agent = bestie.assignedAgent;
  const nudge = useBestieNudge();
  const hasNudge = Boolean(nudge);
  const hasUnread = useBestieHasUnreadMessage();
  const lastAutoOpenedNudgeIdRef = React.useRef<string | null>(null);
  const popoverSize = useBestiePopoverSize();
  const listsCollapsed = useBestiePopoverListsCollapsed();
  const minHeightPx = bestiePopoverMinHeightPx(listsCollapsed);
  const viewportMaxHeightPx = bestiePopoverViewportMaxHeightPx();
  // Effective height is always within the viewport ceiling so drag starts from
  // what the user sees (not a stale full-height persisted value).
  const heightPx = Math.min(
    Math.max(popoverSize.maxHeightPx, minHeightPx),
    viewportMaxHeightPx,
  );
  const [isResizing, setIsResizing] = React.useState(false);
  const dragRef = React.useRef<{
    kind: "width" | "height";
    pointerId: number;
    startX: number;
    startY: number;
    startWidth: number;
    startHeight: number;
    minHeightPx: number;
    parked: boolean;
  } | null>(null);

  // Keep attention store in sync with popover open state.
  React.useEffect(() => {
    setBestiePopoverOpen(open);
    return () => setBestiePopoverOpen(false);
  }, [open]);

  // Lists must start collapsed on every open — ignore a persisted expand from
  // the previous session (toggle still works while open).
  React.useEffect(() => {
    if (!open) return;
    setBestiePopoverListsCollapsed(true);
  }, [open]);

  // Migrate / re-clamp persisted height to 85% viewport + gutter when open
  // (and when the window resizes) so oversized stores can be dragged smaller.
  React.useEffect(() => {
    if (!open) return;
    const clampToViewport = () => {
      // Empty patch re-normalizes the live stored size against current vh.
      setBestiePopoverSize({}, { minHeightPx });
    };
    clampToViewport();
    window.addEventListener("resize", clampToViewport);
    return () => window.removeEventListener("resize", clampToViewport);
  }, [minHeightPx, open]);

  // Auto-open when a *due reminder* nudge fires (distinct from todos check-in).
  React.useEffect(() => {
    if (!agent || !nudge || nudge.reason !== "due-reminder") return;
    if (lastAutoOpenedNudgeIdRef.current === nudge.id) return;
    lastAutoOpenedNudgeIdRef.current = nudge.id;
    setOpen(true);
  }, [agent, nudge]);

  // ⌘B / Ctrl+B — toggle Assistant popover (with or without an assigned agent).
  // detail.open === true forces open (sidebar no-agent path).
  React.useEffect(() => {
    function onShortcut(event: Event) {
      const detail = (event as CustomEvent<{ open?: boolean }>).detail;
      if (detail?.open === true) {
        setOpen(true);
        return;
      }
      setOpen((current) => !current);
    }
    window.addEventListener(BESTIE_POPOVER_SHORTCUT_EVENT, onShortcut);
    return () =>
      window.removeEventListener(BESTIE_POPOVER_SHORTCUT_EVENT, onShortcut);
  }, []);

  const dragListenersRef = React.useRef<{
    move: (event: PointerEvent) => void;
    up: (event: PointerEvent) => void;
    target: Element | null;
  } | null>(null);

  const endDrag = React.useCallback((target?: Element | null) => {
    const drag = dragRef.current;
    const listeners = dragListenersRef.current;
    dragListenersRef.current = null;
    if (listeners) {
      window.removeEventListener("pointermove", listeners.move);
      window.removeEventListener("pointerup", listeners.up);
      target = target ?? listeners.target;
    }
    if (!drag) return;
    dragRef.current = null;
    setIsResizing(false);
    if (drag.parked) {
      releaseNativeWebviewModalPark();
    }
    if (
      target &&
      "hasPointerCapture" in target &&
      typeof (target as Element).hasPointerCapture === "function" &&
      (target as Element).hasPointerCapture(drag.pointerId)
    ) {
      try {
        (target as Element).releasePointerCapture(drag.pointerId);
      } catch {
        // already released
      }
    }
    document.body.style.cursor = "";
    document.body.style.userSelect = "";
  }, []);

  const applyDrag = React.useCallback((event: PointerEvent) => {
    const drag = dragRef.current;
    if (!drag) return;
    if (drag.kind === "width") {
      // The right edge follows the pointer: dragging right grows width.
      const next = drag.startWidth + (event.clientX - drag.startX);
      setBestiePopoverSize({
        widthPx: Math.min(
          BESTIE_POPOVER_MAX_WIDTH_PX,
          Math.max(BESTIE_POPOVER_MIN_WIDTH_PX, next),
        ),
      });
      return;
    }
    // side=top: dragging the top edge upward grows height.
    const next = drag.startHeight + (drag.startY - event.clientY);
    const viewportMax = bestiePopoverViewportMaxHeightPx();
    setBestiePopoverSize(
      {
        maxHeightPx: Math.min(
          BESTIE_POPOVER_MAX_MAX_HEIGHT_PX,
          viewportMax,
          Math.max(drag.minHeightPx, next),
        ),
      },
      { minHeightPx: drag.minHeightPx },
    );
  }, []);

  const startDrag = React.useCallback(
    (
      event: React.PointerEvent<HTMLDivElement>,
      kind: "width" | "height",
    ) => {
      event.preventDefault();
      event.stopPropagation();
      // End any prior gesture before starting a new one.
      endDrag(event.currentTarget);
      // Park natives only for the gesture so pinned sites stay visible while
      // idle, but pointer tracking is not eaten by WKWebViews mid-drag.
      acquireNativeWebviewModalPark();
      const target = event.currentTarget;
      try {
        target.setPointerCapture(event.pointerId);
      } catch {
        // capture unsupported — window listeners + park still help
      }
      document.body.style.cursor = kind === "width" ? "ew-resize" : "ns-resize";
      document.body.style.userSelect = "none";
      dragRef.current = {
        kind,
        pointerId: event.pointerId,
        startX: event.clientX,
        startY: event.clientY,
        startWidth: popoverSize.widthPx,
        startHeight: heightPx,
        minHeightPx,
        parked: true,
      };
      setIsResizing(true);

      const onMove = (moveEvent: PointerEvent) => {
        if (moveEvent.pointerId !== event.pointerId) return;
        // If the OS dropped the gesture without pointerup (native steal), stop.
        if (moveEvent.buttons === 0) {
          endDrag(target);
          return;
        }
        applyDrag(moveEvent);
      };
      const onUp = (upEvent: PointerEvent) => {
        if (upEvent.pointerId !== event.pointerId) return;
        endDrag(target);
      };
      dragListenersRef.current = { move: onMove, up: onUp, target };
      // Window listeners keep tracking after the pointer leaves the handle
      // (and after setPointerCapture is lost when natives steal the gesture).
      // Do NOT end on pointercancel — parking WKWebViews can synthesize cancel
      // and would abort width drag immediately.
      window.addEventListener("pointermove", onMove);
      window.addEventListener("pointerup", onUp);
    },
    [applyDrag, endDrag, heightPx, minHeightPx, popoverSize.widthPx],
  );

  // Safety: if the popover closes mid-drag, release park/capture.
  React.useEffect(() => {
    if (open) return;
    endDrag();
  }, [endDrag, open]);

  React.useEffect(() => {
    return () => {
      endDrag();
    };
  }, [endDrag]);

  const ariaLabel = !agent
    ? "Open Assistant"
    : hasNudge
      ? `Open Assistant chat with ${agent.name} (check-in waiting)`
      : hasUnread
        ? `Open Assistant chat with ${agent.name} (new message)`
        : `Open Assistant chat with ${agent.name}`;

  return (
    <Popover onOpenChange={setOpen} open={open}>
      <PopoverTrigger asChild>
        <button
          aria-label={ariaLabel}
          className={cn(
            "relative flex max-w-[42%] shrink-0 items-center gap-1.5 rounded-full outline-hidden focus-visible:ring-2 focus-visible:ring-ring",
            className,
          )}
          data-testid="bestie-profile-trigger"
          onClick={(event) => event.stopPropagation()}
          type="button"
        >
          <span className="relative inline-flex shrink-0">
            {agent && hasUnread && !open && !hasNudge ? (
              <span
                aria-hidden="true"
                className="assistant-unread-ring pointer-events-none absolute -inset-1 rounded-full"
                data-testid="bestie-unread-ring"
              />
            ) : null}
            <BestieTriggerVisual
              agent={agent}
              className="h-8 w-8"
              compact
              imageDraggable={false}
            />
            {agent && hasNudge ? (
              <span
                aria-hidden="true"
                className="absolute -right-0.5 -top-0.5 flex h-3.5 min-w-3.5 items-center justify-center rounded-full bg-amber-500 px-0.5 text-[9px] font-bold leading-none text-amber-950 ring-2 ring-sidebar"
                data-testid="bestie-nudge-badge"
              >
                !
              </span>
            ) : null}
          </span>
          {agent ? (
            <span
              className="min-w-0 truncate text-xs font-medium text-sidebar-foreground"
              data-testid="bestie-profile-agent-name"
            >
              {agent.name}
            </span>
          ) : null}
        </button>
      </PopoverTrigger>
      <PopoverContent
        align="end"
        className="relative z-[300] flex w-auto max-w-none flex-col overflow-visible p-4"
        onClick={(event) => event.stopPropagation()}
        onInteractOutside={(event) => {
          if (isResizing || dragRef.current) {
            event.preventDefault();
          }
        }}
        onOpenAutoFocus={(event) => {
          const content = event.currentTarget;
          if (!(content instanceof HTMLElement)) return;
          const target = content.querySelector(
            "[data-testid='bestie-composer'], [data-testid='bestie-assign-agent']",
          );
          if (target instanceof HTMLElement) {
            event.preventDefault();
            target.focus();
          }
        }}
        onPointerDownOutside={(event) => {
          if (isResizing || dragRef.current) {
            event.preventDefault();
          }
        }}
        side="top"
        sideOffset={10}
        style={
          {
            // Keep a real viewport gutter even when Radix reports the full
            // available client height/width. 6rem matches
            // BESTIE_POPOVER_VIEWPORT_GUTTER_PX so short displays never fill.
            width: `min(${popoverSize.widthPx}px, calc(100vw - 2rem))`,
            height: `min(${heightPx}px, calc(100vh - 6rem))`,
            minWidth: `min(${BESTIE_POPOVER_MIN_WIDTH_PX}px, calc(100vw - 2rem))`,
            maxWidth: `min(${BESTIE_POPOVER_MAX_WIDTH_PX}px, calc(100vw - 2rem))`,
            minHeight: `min(${minHeightPx}px, calc(100vh - 6rem))`,
            maxHeight: `min(${viewportMaxHeightPx}px, ${BESTIE_POPOVER_MAX_MAX_HEIGHT_PX}px, var(--radix-popover-content-available-height, calc(100vh - 6rem)), calc(100vh - 6rem))`,
            ["--bestie-popover-max-h" as string]: `${heightPx}px`,
          } as React.CSSProperties
        }
      >
        <div className="flex min-h-0 flex-1 flex-col">
          <BestiePopover onRequestClose={() => setOpen(false)} />
        </div>
        {/* Resize handles AFTER content so they stay above chat/Lists hit
            targets. Keep width on the right edge so the grab starts on the
            React surface instead of the native window edge zone; window
            pointermove + brief native park keep tracking after the pointer
            crosses WKWebViews. w-auto + max-w-none override PopoverContent's
            default w-72. */}
        <div
          aria-label="Resize Assistant width"
          className="absolute bottom-2 right-0 top-2 z-30 w-3 cursor-ew-resize touch-none rounded-full bg-transparent hover:bg-border/80"
          data-testid="bestie-popover-resize-width"
          onPointerDown={(event) => startDrag(event, "width")}
        />
        <div
          aria-label="Resize Assistant height"
          className="absolute left-2 right-2 top-0 z-30 h-3 cursor-ns-resize touch-none rounded-full bg-transparent hover:bg-border/80"
          data-testid="bestie-popover-resize-height"
          onPointerDown={(event) => startDrag(event, "height")}
        />
      </PopoverContent>
    </Popover>
  );
}
