/**
 * Normalize a relay URL to ws(s):// form and probe reachability.
 */

/** Normalize a user-entered relay URL to ws(s):// form. Returns null if invalid. */
export function normalizeRelayUrl(input: string): string | null {
  const trimmed = input.trim().replace(/\/+$/, "");
  if (!trimmed) return null;

  // Already ws(s)://
  if (trimmed.startsWith("wss://") || trimmed.startsWith("ws://")) {
    try {
      new URL(trimmed);
      return trimmed;
    } catch {
      return null;
    }
  }

  // Convert https → wss, http → ws
  if (trimmed.startsWith("https://")) {
    const wsUrl = `wss://${trimmed.slice(8)}`;
    try {
      new URL(wsUrl);
      return wsUrl;
    } catch {
      return null;
    }
  }
  if (trimmed.startsWith("http://")) {
    const wsUrl = `ws://${trimmed.slice(7)}`;
    try {
      new URL(wsUrl);
      return wsUrl;
    } catch {
      return null;
    }
  }

  return null;
}

/**
 * Probe whether a WebSocket relay is reachable. Opens a connection with a
 * timeout; resolves `true` if the socket opens, `false` on timeout/error.
 * The socket is always closed before returning.
 */
export function probeRelayReachable(
  wsUrl: string,
  timeoutMs = 4000,
): { promise: Promise<boolean>; cancel: () => void } {
  let socket: WebSocket | null = null;
  let timeoutId: ReturnType<typeof setTimeout> | null = null;
  let settled = false;

  const promise = new Promise<boolean>((resolve) => {
    function settle(result: boolean) {
      if (settled) return;
      settled = true;
      if (timeoutId !== null) clearTimeout(timeoutId);
      try {
        socket?.close();
      } catch {
        /* ignore */
      }
      socket = null;
      resolve(result);
    }

    try {
      socket = new WebSocket(wsUrl);
      socket.onopen = () => settle(true);
      socket.onerror = () => settle(false);
      socket.onclose = () => settle(false);
      timeoutId = setTimeout(() => settle(false), timeoutMs);
    } catch {
      settle(false);
    }
  });

  return {
    promise,
    cancel() {
      if (settled) return;
      settled = true;
      if (timeoutId !== null) clearTimeout(timeoutId);
      try {
        socket?.close();
      } catch {
        /* ignore */
      }
      socket = null;
    },
  };
}
