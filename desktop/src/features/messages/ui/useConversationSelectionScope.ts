import * as React from "react";

import {
  closestPane,
  isWithinInteractiveText,
  resolveClampBoundary,
  shouldClampSelectionToPane,
  type PaneBoundary,
} from "@/features/messages/lib/selection/conversationSelectionScope";

/**
 * Clip native drag-selections to the conversation pane they began in.
 *
 * block/buzz#4077: a selection starting in the channel can drag into the
 * thread (or vice versa) and sweep up hover action bars, quick reactions,
 * reaction pills, emoji pickers, and popovers along the way. The shared
 * conversation flex-root mounts this hook; when a selectionchange moves the
 * focus end out of the anchor pane, we collapse it back to the pane's edge.
 *
 * The decision logic is pure and lives in
 * `lib/selection/conversationSelectionScope.ts`; this hook is only the DOM
 * wiring (listeners, refs, and the rAF-coalesced selection mutation).
 *
 * Nothing here runs during IME composition, when the selection is inside an
 * interactive text surface (composer / find bar), or when a top-level dialog
 * owns focus — in those cases selection is left entirely to the element.
 */
export function useConversationSelectionScope(
  containerRef: React.RefObject<HTMLElement | null>,
): void {
  // The pane the active selection started in, remembered across change events
  // so a long drag only needs one boundary decision.
  const anchorPaneRef = React.useRef<Element | null>(null);
  const frameRef = React.useRef(0);

  React.useEffect(() => {
    if (typeof document === "undefined") return undefined;
    const container = containerRef.current;
    if (!container) return undefined;

    const dropAnchor = () => {
      anchorPaneRef.current = null;
    };

    const clampToPaneBoundary = (selection: Selection) => {
      const anchorPane = anchorPaneRef.current;
      if (!anchorPane) return;
      const focusNode = selection.focusNode;
      if (!focusNode || focusNode.nodeType !== /* ELEMENT_NODE */ 1) {
        // Focus on a text node — treat its parent element as the focus.
        const parent = (focusNode as Text).parentElement;
        if (!parent) return;
        applyClamp(selection, anchorPane, parent);
        return;
      }
      applyClamp(selection, anchorPane, focusNode as unknown as Element);
    };

    const applyClamp = (
      selection: Selection,
      anchorPane: Element,
      focusElement: Element,
    ) => {
      // The pure rule decides whether to clamp and to which boundary.
      const pane = shouldClampSelectionToPane(anchorPane, focusElement);
      if (!pane) return;
      const boundary: PaneBoundary = resolveClampBoundary(pane, focusElement);
      const anchorNode = selection.anchorNode;
      if (!anchorNode) return;
      // Collapse the moving end onto the pane's boundary as a single caret
      // operation so the visible selection never paints outside the pane.
      // `setBaseAndExtent` keeps the anchor end fixed, preserving direction.
      try {
        const clampNode =
          boundary === "after"
            ? findLastSelectableDescendant(pane)
            : findFirstSelectableDescendant(pane);
        if (!clampNode) return;
        const offset =
          boundary === "after"
            ? clampNode.nodeType === /* TEXT_NODE */ 3
              ? (clampNode as Text).length
              : clampNode.childNodes.length
            : 0;
        selection.setBaseAndExtent(
          anchorNode,
          selection.anchorOffset,
          clampNode,
          offset,
        );
      } catch {
        // Selection DOM can move mid-drag; a stale node is recoverable next frame.
      }
    };

    const onSelectionChange = () => {
      if (frameRef.current) return;
      frameRef.current = window.requestAnimationFrame(() => {
        frameRef.current = 0;
        const selection = document.getSelection();
        if (!selection || selection.isCollapsed) {
          // A collapsed caret means the drag ended — release the anchor so a
          // later fresh drag re-anchors to whichever pane it begins in.
          if (selection && selection.isCollapsed) dropAnchor();
          return;
        }
        const anchorPane = anchorPaneRef.current;
        if (!anchorPane) {
          // First change for a fresh drag: anchor to the pane that owns the
          // selection's anchor end. If the drag began outside a conversation
          // pane there is nothing to scope.
          const anchorNode = selection.anchorNode;
          const anchorElement =
            anchorNode && anchorNode.nodeType === 1
              ? (anchorNode as unknown as Element)
              : ((anchorNode as Text | null)?.parentElement ?? null);
          if (anchorElement) {
            const pane = closestPane(anchorElement);
            if (
              pane &&
              !isWithinInteractiveText(anchorElement)
            ) {
              anchorPaneRef.current = pane as unknown as Element;
            }
          }
          return;
        }
        clampToPaneBoundary(selection);
      });
    };

    document.addEventListener("selectionchange", onSelectionChange);
    return () => {
      document.removeEventListener("selectionchange", onSelectionChange);
      if (frameRef.current) {
        window.cancelAnimationFrame(frameRef.current);
        frameRef.current = 0;
      }
      anchorPaneRef.current = null;
    };
  }, [containerRef]);
}

/**
 * Last descendant of `root` that can accept a caret — a text node with
 * content, or an element with children. Pure depth-first on the live DOM.
 */
function findLastSelectableDescendant(root: Element): Node | null {
  const children = root.childNodes;
  for (let i = children.length - 1; i >= 0; i -= 1) {
    const node = children[i];
    if (node.nodeType === /* TEXT_NODE */ 3) {
      if ((node as Text).length > 0) return node;
      continue;
    }
    if (node.nodeType === /* ELEMENT_NODE */ 1) {
      // Skip excluded chrome — its text should not be clamped onto.
      if (
        typeof (node as Element).matches === "function" &&
        (node as Element).matches("[data-selection-exclude]")
      ) {
        continue;
      }
      const nested = findLastSelectableDescendant(node as Element);
      if (nested) return nested;
    }
  }
  return root.nodeType === /* TEXT_NODE */ 3 ? root : null;
}

/** Mirror of `findLastSelectableDescendant` for the leading edge. */
function findFirstSelectableDescendant(root: Element): Node | null {
  const children = root.childNodes;
  for (let i = 0; i < children.length; i += 1) {
    const node = children[i];
    if (node.nodeType === /* TEXT_NODE */ 3) {
      if ((node as Text).length > 0) return node;
      continue;
    }
    if (node.nodeType === /* ELEMENT_NODE */ 1) {
      if (
        typeof (node as Element).matches === "function" &&
        (node as Element).matches("[data-selection-exclude]")
      ) {
        continue;
      }
      const nested = findFirstSelectableDescendant(node as Element);
      if (nested) return nested;
    }
  }
  return root.nodeType === /* TEXT_NODE */ 3 ? root : null;
}
