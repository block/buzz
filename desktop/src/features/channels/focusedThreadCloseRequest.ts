const listeners = new Set<() => void>();

/** Request dismissal of an open focus-mode thread drawer. */
export function requestFocusedThreadClose(): void {
  for (const listener of listeners) {
    listener();
  }
}

/** Subscribe the active channel surface to focus-mode dismissal requests. */
export function subscribeToFocusedThreadCloseRequest(
  listener: () => void,
): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}
