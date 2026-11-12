import * as React from "react";

/**
 * Tracks whether a composer owns document focus: true while the focused
 * element lives anywhere inside `containerRef` (the composer form) — the
 * editor or the focusable controls of its suggestion overlays.
 *
 * This is the value the autocomplete overlays gate their rendering on. It is
 * deliberately not the editor's own focus state: an overlay gated on editor
 * focus alone unmounts the moment keyboard focus moves from the editor into
 * the overlay's controls, which makes those controls unreachable. Ownership
 * is tracked with `focusout` + `relatedTarget` containment rather than
 * blur/focus pairs so an internal focus move never passes through a false
 * state — a false flicker would unmount the overlay before the control it
 * is handing focus to receives it. Each composer form has its own instance,
 * so focus in one composer never keeps a sibling composer's overlays alive.
 */
export function useComposerFocusOwnership(
  containerRef: React.RefObject<HTMLElement | null>,
): boolean {
  const [ownsFocus, setOwnsFocus] = React.useState(false);

  React.useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    setOwnsFocus(container.contains(document.activeElement));
    const handleFocusIn = () => setOwnsFocus(true);
    const handleFocusOut = (event: FocusEvent) => {
      setOwnsFocus(
        event.relatedTarget instanceof Node &&
          container.contains(event.relatedTarget),
      );
    };
    container.addEventListener("focusin", handleFocusIn);
    container.addEventListener("focusout", handleFocusOut);
    return () => {
      container.removeEventListener("focusin", handleFocusIn);
      container.removeEventListener("focusout", handleFocusOut);
    };
  }, [containerRef]);

  return ownsFocus;
}
