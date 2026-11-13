/**
 * Mirrors `document.visibilityState` onto `<html data-window-hidden>` so CSS
 * can pause every infinite animation (spinners, shimmers, bee wings, grainient
 * orbit, skeleton pulses) while the window is minimized, occluded or on
 * another virtual desktop. See `globals/hidden-pause.css`.
 *
 * Returns a cleanup function.
 */
export function startPauseAnimationsWhenHidden(): () => void {
  if (typeof document === "undefined") return () => {};

  const root = document.documentElement;
  const sync = () => {
    if (document.visibilityState === "hidden") {
      root.setAttribute("data-window-hidden", "");
    } else {
      root.removeAttribute("data-window-hidden");
    }
  };
  sync();
  document.addEventListener("visibilitychange", sync);
  return () => {
    document.removeEventListener("visibilitychange", sync);
    root.removeAttribute("data-window-hidden");
  };
}
