import * as React from "react";

import type { AppView } from "@/app/AppShell.helpers";
import { cn } from "@/shared/lib/cn";
import {
  getPlatformKeys,
  getShortcutHintsForView,
  type KeyboardShortcut,
} from "@/shared/lib/keyboard-shortcuts";

type ShortcutStatusBarProps = {
  view: AppView;
};

/**
 * Split a platform key string into individual key chips. Mirrors the split in
 * the settings cheatsheet: break on "+" but keep a trailing "+" as its own key
 * (e.g. zoom-in's "⌘+").
 */
function splitKeys(keys: string): string[] {
  return keys
    .split(/(?<!\+)\+(?!\s*$)/)
    .map((part) => part.trim())
    .filter(Boolean);
}

function HintKeys({ shortcut }: { shortcut: KeyboardShortcut }) {
  const parts = splitKeys(getPlatformKeys(shortcut));

  return (
    <span className="flex items-center gap-0.5">
      {parts.map((part) => (
        <kbd
          className="inline-flex h-5 min-w-5 items-center justify-center rounded border border-border/60 bg-muted/50 px-1.5 font-mono text-2xs text-muted-foreground"
          key={part}
        >
          {part}
        </kbd>
      ))}
    </span>
  );
}

/**
 * Global, edge-to-edge status bar pinned to the bottom of the app window. It
 * surfaces the keyboard shortcuts relevant to the current view as a
 * discoverability aid. Content is resolved from the shortcut registry via
 * {@link getShortcutHintsForView} and is contextual to `view`.
 *
 * The bar renders nothing when a view resolves to no hints, so callers can
 * mount it unconditionally (gating for settings/huddle happens upstream).
 */
export function ShortcutStatusBar({ view }: ShortcutStatusBarProps) {
  const hints = getShortcutHintsForView(view);
  if (hints.length === 0) return null;

  return (
    <footer
      aria-label="Keyboard shortcuts"
      className={cn(
        "flex h-12 shrink-0 select-none items-center justify-center gap-3",
        "border-t border-border/40 bg-sidebar/60 px-3 text-xs text-muted-foreground",
      )}
      data-testid="shortcut-status-bar"
      role="contentinfo"
    >
      {hints.map((shortcut, index) => (
        <React.Fragment key={shortcut.id}>
          {index > 0 ? (
            <span aria-hidden="true" className="text-muted-foreground/40">
              ·
            </span>
          ) : null}
          <span className="flex items-center gap-1.5">
            <HintKeys shortcut={shortcut} />
            <span>{shortcut.label}</span>
          </span>
        </React.Fragment>
      ))}
    </footer>
  );
}
