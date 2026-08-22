const listeners = new Set<() => void>();

/**
 * Request dismissal of the channel's open cover drawer.
 *
 * One channel of a channel pane is covered at a time (focus-mode thread or
 * agent activity), so this needs no discriminator — whichever drawer is open
 * subscribes and closes.
 */
export function requestCoverDrawerClose(): void {
  for (const listener of listeners) {
    listener();
  }
}

/** Subscribe the active cover drawer to external dismissal requests. */
export function subscribeToCoverDrawerCloseRequest(
  listener: () => void,
): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}
