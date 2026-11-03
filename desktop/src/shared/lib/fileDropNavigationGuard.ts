function preventFileDropNavigation(event: DragEvent) {
  if (event.dataTransfer?.types.includes("Files")) {
    event.preventDefault();
  }
}

/**
 * Stops a file dropped anywhere that is not a drop target from navigating the
 * webview to that file, which would replace the whole window with it.
 *
 * Installed once at bootstrap so it covers every screen, including the ones
 * rendered before the app shell (onboarding, loading and recovery gates).
 * Only file drags are cancelled, so dragging text into inputs still works, and
 * drop targets run their own handlers first because the window listener sees
 * the event last. Returns a function that removes the listeners.
 */
export function installFileDropNavigationGuard(
  target: Pick<Window, "addEventListener" | "removeEventListener"> = window,
): () => void {
  target.addEventListener("dragover", preventFileDropNavigation);
  target.addEventListener("drop", preventFileDropNavigation);
  return () => {
    target.removeEventListener("dragover", preventFileDropNavigation);
    target.removeEventListener("drop", preventFileDropNavigation);
  };
}
