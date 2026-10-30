import * as React from "react";

type Handler = (payload: unknown) => void;

const handlers = new Map<string, Set<Handler>>();

/**
 * Asks a mounted component to open UI the composer does not own, such as the
 * member list or the set-status dialog. Returns false when no component
 * handles `name`, so the command can fail visibly instead of doing nothing.
 */
export function requestSlashCommandUi(
  name: string,
  payload?: unknown,
): boolean {
  const listeners = handlers.get(name);
  if (!listeners || listeners.size === 0) return false;
  for (const listener of [...listeners]) listener(payload);
  return true;
}

/** Mount in the component that owns the UI named `name`. */
export function useSlashCommandUiHandler(
  name: string,
  handler: (payload: unknown) => void,
) {
  const handlerRef = React.useRef(handler);
  handlerRef.current = handler;
  React.useEffect(() => {
    const listener: Handler = (payload) => handlerRef.current(payload);
    let listeners = handlers.get(name);
    if (!listeners) {
      listeners = new Set();
      handlers.set(name, listeners);
    }
    listeners.add(listener);
    return () => {
      listeners.delete(listener);
      if (listeners.size === 0) handlers.delete(name);
    };
  }, [name]);
}
