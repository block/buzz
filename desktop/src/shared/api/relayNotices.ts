export type RelayNoticeListener = (message: string) => void;

const relayNoticeListeners = new Set<RelayNoticeListener>();

export function subscribeToRelayNotices(
  listener: RelayNoticeListener,
): () => void {
  relayNoticeListeners.add(listener);
  return () => {
    relayNoticeListeners.delete(listener);
  };
}

export function emitRelayNotice(message: string): void {
  if (message.trim().length === 0) {
    return;
  }

  for (const listener of relayNoticeListeners) {
    try {
      listener(message);
    } catch (error) {
      console.error("Failed to deliver relay NOTICE", error);
    }
  }
}

export function relayNoticeMessageFromFrame(frame: unknown): string | null {
  if (!Array.isArray(frame) || frame[0] !== "NOTICE") {
    return null;
  }

  const message = frame[1];
  if (typeof message !== "string" || message.trim().length === 0) {
    return null;
  }

  return message;
}

/**
 * Consume a NIP-01 NOTICE frame and surface it to registered UI listeners.
 * Returns true for all NOTICE frames, including malformed/blank ones, so callers
 * do not treat relay warnings as socket failures or ordinary protocol frames.
 */
export function handleRelayNoticeFrame(
  frame: unknown,
  emit: RelayNoticeListener = emitRelayNotice,
): boolean {
  if (!Array.isArray(frame) || frame[0] !== "NOTICE") {
    return false;
  }

  const message = relayNoticeMessageFromFrame(frame);
  if (message) {
    emit(message);
  }

  return true;
}

export function clearRelayNoticeListenersForTests(): void {
  relayNoticeListeners.clear();
}
