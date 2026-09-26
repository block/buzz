let currentRequest: object | undefined;

/** Supersede pending message-link lookups across all mounted hook instances. */
export function beginMessageLinkRequest(): () => boolean {
  const request = {};
  currentRequest = request;
  return () => currentRequest === request;
}

/** Invalidate pending message-link routing before switching communities. */
export function resetMessageLinkRequests(): void {
  currentRequest = undefined;
}
