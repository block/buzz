import * as React from "react";

/**
 * App-wide display style.
 *
 * - `standard` — the default sidebar + channel pane layout.
 * - `developer` — a prompt-first, terminal-style surface: one composer that
 *   spawns a channel per prompt with an agent tagged (Tab cycles the target
 *   agent), plus keyboard-driven session navigation.
 *
 * Session-only on purpose: the app always launches in standard mode, and dev
 * mode is opted into per session (⌘⇧D or the top-chrome Dev Mode button).
 */
export type DisplayStyle = "standard" | "developer";

const DEFAULT_DISPLAY_STYLE: DisplayStyle = "standard";

const listeners = new Set<() => void>();

let displayStyle: DisplayStyle = DEFAULT_DISPLAY_STYLE;

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function getSnapshot(): DisplayStyle {
  return displayStyle;
}

function getServerSnapshot(): DisplayStyle {
  return DEFAULT_DISPLAY_STYLE;
}

/** Read the persisted display style outside of React. */
export function getDisplayStyle(): DisplayStyle {
  return displayStyle;
}

/** Update the display style and notify all subscribed components. */
export function setDisplayStyle(style: DisplayStyle): void {
  displayStyle = style;

  for (const listener of listeners) {
    listener();
  }
}

export function toggleDisplayStyle(): void {
  setDisplayStyle(displayStyle === "developer" ? "standard" : "developer");
}

export function useDisplayStyle(): DisplayStyle {
  return React.useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);
}
