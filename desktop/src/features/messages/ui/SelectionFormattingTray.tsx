import * as React from "react";
import { createPortal } from "react-dom";
import type { EditorView } from "@tiptap/pm/view";
import type { Editor } from "@tiptap/react";

import { cn } from "@/shared/lib/cn";
import { getMountedView } from "../lib/mountedEditorView";
import { FormattingToolbar } from "./FormattingToolbar";
import { getMountedEditorDom } from "./selectionFormattingTrayEditorDom";

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

function getSelectionRect(editor: Editor, view: EditorView): DOMRect | null {
  const { from, to } = editor.state.selection;

  try {
    const range = document.createRange();
    const start = view.domAtPos(from);
    const end = view.domAtPos(to);
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

  try {
    const startCoords = view.coordsAtPos(from);
    const endCoords = view.coordsAtPos(to);
    const left = Math.min(startCoords.left, endCoords.left);
    const right = Math.max(startCoords.right, endCoords.right);
    const top = Math.min(startCoords.top, endCoords.top);
    const bottom = Math.max(startCoords.bottom, endCoords.bottom);

    if (right <= left && bottom <= top) return null;
    return new DOMRect(left, top, Math.max(1, right - left), bottom - top);
  } catch {
    // The view detached mid-measurement; leave the tray hidden.
    return null;
  }
}

function getTrayPosition(
  editor: Editor,
  view: EditorView,
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

  const rect = getSelectionRect(editor, view);
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
  const suppressRightClickUpdatesRef = React.useRef(false);
  const trayRef = React.useRef<HTMLDivElement | null>(null);
  const [trayWidth, setTrayWidth] = React.useState(0);
  // The view attaches and detaches independently of the editor, so track it as
  // state rather than reading `editor.view` at wiring time.
  const [mountedView, setMountedView] = React.useState<EditorView | null>(null);

  const cancelScheduledUpdate = React.useCallback(() => {
    if (rafRef.current === null) return;
    window.cancelAnimationFrame(rafRef.current);
    rafRef.current = null;
  }, []);

  const updatePosition = React.useCallback(() => {
    if (
      suppressRightClickUpdatesRef.current ||
      !editor ||
      !getMountedEditorDom(editor) ||
      disabled ||
      !editor.isEditable ||
      !editor.isFocused
    ) {
      setPosition(null);
      return;
    }
    if (!mountedView) {
      setPosition(null);
      return;
    }
    setPosition(getTrayPosition(editor, mountedView, trayWidth));
  }, [disabled, editor, mountedView, trayWidth]);

  const scheduleUpdate = React.useCallback(() => {
    if (suppressRightClickUpdatesRef.current) {
      cancelScheduledUpdate();
      setPosition(null);
      return;
    }
    cancelScheduledUpdate();
    rafRef.current = window.requestAnimationFrame(() => {
      rafRef.current = null;
      updatePosition();
    });
  }, [cancelScheduledUpdate, updatePosition]);

  // React can reconnect these effects while the composer subtree is hidden, at
  // which point `EditorContent` has already torn the view down. Follow tiptap's
  // mount/unmount events instead of reading the throwing view proxy on demand.
  React.useEffect(() => {
    if (!editor) {
      setMountedView(null);
      return;
    }

    const syncView = () => setMountedView(getMountedView(editor));
    syncView();
    editor.on("mount", syncView);
    editor.on("unmount", syncView);

    return () => {
      editor.off("mount", syncView);
      editor.off("unmount", syncView);
    };
  }, [editor]);

  React.useEffect(() => {
    suppressRightClickUpdatesRef.current = false;

    if (!editor || !mountedView) {
      cancelScheduledUpdate();
      setPosition(null);
      return;
    }

    let editorDom: HTMLElement | null = null;
    const hide = () => setPosition(null);
    const handleContextMenu = () => {
      suppressRightClickUpdatesRef.current = true;
      cancelScheduledUpdate();
      setPosition(null);
    };
    const clearSuppression = () => {
      suppressRightClickUpdatesRef.current = false;
      scheduleUpdate();
    };
    const handlePointerDown = (event: PointerEvent) => {
      if (event.button === 0) clearSuppression();
    };

    const detachEditorDom = () => {
      if (!editorDom) return;
      editorDom.removeEventListener("contextmenu", handleContextMenu);
      editorDom.removeEventListener("pointerdown", handlePointerDown);
      editorDom.removeEventListener("keydown", clearSuppression);
      editorDom = null;
    };

    const attachEditorDom = () => {
      const nextEditorDom = getMountedEditorDom(editor);
      if (!nextEditorDom || nextEditorDom === editorDom) return;
      detachEditorDom();
      editorDom = nextEditorDom;
      editorDom.addEventListener("contextmenu", handleContextMenu);
      editorDom.addEventListener("pointerdown", handlePointerDown);
      editorDom.addEventListener("keydown", clearSuppression);
      scheduleUpdate();
    };

    editor.on("mount", attachEditorDom);
    editor.on("unmount", detachEditorDom);
    editor.on("selectionUpdate", scheduleUpdate);
    editor.on("transaction", scheduleUpdate);
    editor.on("focus", scheduleUpdate);
    editor.on("blur", hide);
    window.addEventListener("resize", scheduleUpdate);
    window.addEventListener("scroll", scheduleUpdate, true);
    attachEditorDom();

    return () => {
      cancelScheduledUpdate();
      editor.off("mount", attachEditorDom);
      editor.off("unmount", detachEditorDom);
      editor.off("selectionUpdate", scheduleUpdate);
      editor.off("transaction", scheduleUpdate);
      editor.off("focus", scheduleUpdate);
      editor.off("blur", hide);
      detachEditorDom();
      window.removeEventListener("resize", scheduleUpdate);
      window.removeEventListener("scroll", scheduleUpdate, true);
    };
  }, [cancelScheduledUpdate, editor, mountedView, scheduleUpdate]);

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
      data-buzz-selection-formatting-tray
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
