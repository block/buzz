import type { ConnectionState } from "@/shared/api/relayClientShared";

/**
 * Small observable for `ConnectionState`. Kept separate from the relay client
 * so the file-size budget stays sane and so the contract is unit-testable
 * without dragging the whole session manager into scope.
 *
 * Semantics:
 *  - `set(next)` is a no-op if `next === current` (no duplicate events).
 *  - New subscribers receive the current state immediately (synchronously),
 *    so React hooks don't need a separate "getState" call to seed their UI.
 *  - Listener exceptions are caught and logged, never propagated back into
 *    the caller of `set`.
 */
export class RelayConnectionStateEmitter {
  private state: ConnectionState;
  private listeners = new Set<(state: ConnectionState) => void>();

  constructor(initial: ConnectionState = "idle") {
    this.state = initial;
  }

  get(): ConnectionState {
    return this.state;
  }

  set(next: ConnectionState): void {
    if (this.state === next) {
      return;
    }
    this.state = next;
    for (const listener of this.listeners) {
      try {
        listener(next);
      } catch (error) {
        console.error("Failed to deliver relay connection state", error);
      }
    }
  }

  subscribe(listener: (state: ConnectionState) => void): () => void {
    this.listeners.add(listener);
    try {
      listener(this.state);
    } catch (error) {
      console.error("Failed to deliver initial relay connection state", error);
    }
    return () => {
      this.listeners.delete(listener);
    };
  }

  /** Drop all listeners. Called on workspace teardown to match the legacy
   *  pattern used for the reconnect listener set. */
  clear(): void {
    this.listeners.clear();
  }
}
