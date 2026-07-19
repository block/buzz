/**
 * Tracks closable foreground surfaces currently listening for Escape.
 *
 * Escape has app-wide meaning (mark channel read) *and* surface-local meaning
 * (close the panel above the channel). Window listeners fire in registration
 * order, so the app-level shortcut — registered at mount — would otherwise
 * always win the key over a panel that opened later. Instead of racing,
 * background shortcuts ask "is any closable surface open?" and yield.
 *
 * Nested controls (autocomplete, edit mode) still take priority over the
 * surfaces themselves: they handle Escape on the element and mark it
 * `defaultPrevented`, which every surface listener already respects.
 */
let activeEscapeSurfaceCount = 0;

/** True while at least one closable surface is listening for Escape. */
export function hasActiveEscapeSurface(): boolean {
  return activeEscapeSurfaceCount > 0;
}

/**
 * Registers a closable surface. Returns a release function that must be
 * called exactly once when the surface stops listening (idempotent — extra
 * calls are ignored so a double-cleanup cannot corrupt the count).
 */
export function acquireEscapeSurface(): () => void {
  activeEscapeSurfaceCount += 1;
  let released = false;
  return () => {
    if (released) return;
    released = true;
    activeEscapeSurfaceCount -= 1;
  };
}
