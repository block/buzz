/**
 * Shared constants and backdrop for thread/agent overlay panels
 * that slide in from the right at narrow viewport widths.
 */

/** Base classes for every side panel `<aside>`. */
export const PANEL_BASE_CLASS =
  "relative flex h-full shrink-0 flex-col border-l border-border/80 bg-background";

/**
 * Extra classes applied when the panel is rendered as a floating overlay.
 *
 * Starts below the fixed top chrome (window drag region + global search and
 * channel actions, ~44px tall) so the panel header doesn't collide with it at
 * narrow widths. Matches the inline layout where the header sits below chrome.
 *
 * `h-auto` overrides the base `h-full`: with `fixed top-11 bottom-0`, height is
 * driven by the top/bottom insets. Without it, `h-full` resolves to 100vh and
 * the panel hangs below the viewport (100vh tall, but starting 44px down).
 */
export const PANEL_OVERLAY_CLASS =
  "fixed bottom-0 right-0 top-11 z-40 h-auto shadow-xl max-w-[calc(100vw-2rem)]";

/**
 * Single-column panel headers should render above the local panel backdrop
 * (z-40) but stay below global top chrome controls (z-[45]).
 */
export const PANEL_SINGLE_COLUMN_HEADER_LAYER_CLASS = "z-[41]";

type OverlayPanelBackdropProps = {
  onClose: () => void;
};

export function OverlayPanelBackdrop({ onClose }: OverlayPanelBackdropProps) {
  return (
    <div
      className="fixed inset-0 z-30 bg-black/20"
      onClick={onClose}
      aria-hidden="true"
    />
  );
}
