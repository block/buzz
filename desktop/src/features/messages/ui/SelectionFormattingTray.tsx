import * as React from "react";
import { createPortal } from "react-dom";
import type { Editor } from "@tiptap/react";

import { cn } from "@/shared/lib/cn";
import { FormattingToolbar } from "./FormattingToolbar";

type SelectionFormattingTrayProps = {
  editor: Editor | null;
  disabled?: boolean;
  onLinkButton?: () => void;
};

type TrayPosition = {
  left: number;
  placement: "top" | "bottom";
  top: number;
};

const EDGE_GUTTER = 12;
const SELECTION_OFFSET = 8;
const MIN_SPACE_ABOVE = 44;

function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), max);
}

function getSelectionRect(editor: Editor): DOMRect | null {
  const { from, to } = editor.state.selection;

  try {
    const range = document.createRange();
    const start = editor.view.domAtPos(from);
    const end = editor.view.domAtPos(to);
    range.setStart(start.node, start.offset);
    range.setEnd(end.node, end.offset);

    const clientRects = Array.from(range.getClientRects()).filter(
      (rect) => rect.width > 0 || rect.height > 0,
    );
    const rect = clientRects[0] ?? range.getBoundingClientRect();
    range.detach();

    if (rect.width > 0 || rect.height > 0) return rect;
  } catch {
    // Fall back to the caret coordinates below.
  }

  const startCoords = editor.view.coordsAtPos(from);
  const endCoords = editor.view.coordsAtPos(to);
  const left = Math.min(startCoords.left, endCoords.left);
  const right = Math.max(startCoords.right, endCoords.right);
  const top = Math.min(startCoords.top, endCoords.top);
  const bottom = Math.max(startCoords.bottom, endCoords.bottom);

  if (right <= left && bottom <= top) return null;
  return new DOMRect(left, top, Math.max(1, right - left), bottom - top);
}

function getTrayPosition(
  editor: Editor,
  trayWidth: number,
): TrayPosition | null {
  const { selection } = editor.state;
  if (selection.empty || selection.from === selection.to) return null;

  const selectedText = editor.state.doc.textBetween(
    selection.from,
    selection.to,
    "\n",
    "\n",
  );
  if (selectedText.trim().length === 0) return null;

  const rect = getSelectionRect(editor);
  if (!rect) return null;

  const viewportWidth = window.innerWidth;
  const viewportHeight = window.innerHeight;
  const selectionCenter = rect.left + rect.width / 2;
  const halfTrayWidth = trayWidth / 2;
  const minLeft = Math.min(
    viewportWidth - EDGE_GUTTER,
    EDGE_GUTTER + halfTrayWidth,
  );
  const maxLeft = Math.max(
    EDGE_GUTTER,
    viewportWidth - EDGE_GUTTER - halfTrayWidth,
  );
  const left =
    minLeft <= maxLeft
      ? clamp(selectionCenter, minLeft, maxLeft)
      : viewportWidth / 2;
  const hasRoomAbove = rect.top >= MIN_SPACE_ABOVE;

  if (hasRoomAbove) {
    return {
      left,
      placement: "top",
      top: Math.max(EDGE_GUTTER, rect.top - SELECTION_OFFSET),
    };
  }

  return {
    left,
    placement: "bottom",
    top: Math.min(viewportHeight - EDGE_GUTTER, rect.bottom + SELECTION_OFFSET),
  };
}

export function SelectionFormattingTray({
  editor,
  disabled = false,
  onLinkButton,
}: SelectionFormattingTrayProps) {
  const [position, setPosition] = React.useState<TrayPosition | null>(null);
  const rafRef = React.useRef<number | null>(null);
  const trayRef = React.useRef<HTMLDivElement | null>(null);
  const [trayWidth, setTrayWidth] = React.useState(0);

  const updatePosition = React.useCallback(() => {
    if (!editor || disabled || !editor.isEditable || !editor.isFocused) {
      setPosition(null);
      return;
    }
    setPosition(getTrayPosition(editor, trayWidth));
  }, [disabled, editor, trayWidth]);

  const scheduleUpdate = React.useCallback(() => {
    if (rafRef.current !== null) {
      window.cancelAnimationFrame(rafRef.current);
    }
    rafRef.current = window.requestAnimationFrame(() => {
      rafRef.current = null;
      updatePosition();
    });
  }, [updatePosition]);

  React.useEffect(() => {
    if (!editor) {
      setPosition(null);
      return;
    }

    const hide = () => setPosition(null);

    scheduleUpdate();
    editor.on("selectionUpdate", scheduleUpdate);
    editor.on("transaction", scheduleUpdate);
    editor.on("focus", scheduleUpdate);
    editor.on("blur", hide);
    window.addEventListener("resize", scheduleUpdate);
    window.addEventListener("scroll", scheduleUpdate, true);

    return () => {
      if (rafRef.current !== null) {
        window.cancelAnimationFrame(rafRef.current);
        rafRef.current = null;
      }
      editor.off("selectionUpdate", scheduleUpdate);
      editor.off("transaction", scheduleUpdate);
      editor.off("focus", scheduleUpdate);
      editor.off("blur", hide);
      window.removeEventListener("resize", scheduleUpdate);
      window.removeEventListener("scroll", scheduleUpdate, true);
    };
  }, [editor, scheduleUpdate]);

  React.useLayoutEffect(() => {
    if (!position || !trayRef.current) return;

    const updateTrayWidth = () => {
      const nextWidth = trayRef.current?.getBoundingClientRect().width ?? 0;
      setTrayWidth((currentWidth) =>
        Math.abs(currentWidth - nextWidth) > 1 ? nextWidth : currentWidth,
      );
    };

    updateTrayWidth();

    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(updateTrayWidth);
    observer.observe(trayRef.current);
    return () => observer.disconnect();
  }, [position]);

  if (!position || typeof document === "undefined") return null;

  return createPortal(
    <div
      ref={trayRef}
      className={cn(
        "fixed z-50 max-w-[calc(100vw-1.5rem)] rounded-lg border border-border/80 bg-popover p-1 text-popover-foreground shadow-lg",
        position.placement === "top"
          ? "-translate-x-1/2 -translate-y-full"
          : "-translate-x-1/2",
      )}
      data-testid="selection-formatting-tray"
      onMouseDown={(event) => event.preventDefault()}
      role="toolbar"
      aria-label="Selection formatting"
      style={{ left: position.left, top: position.top }}
    >
      <div className="max-w-full overflow-x-auto">
        <FormattingToolbar
          disabled={disabled}
          editor={editor}
          onLinkButton={onLinkButton}
        />
      </div>
    </div>,
    document.body,
  );
}
