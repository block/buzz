import * as React from "react";

const SHORTCUT_GROUPS: {
  title: string;
  shortcuts: { keys: string; action: string }[];
}[] = [
  {
    title: "composer",
    shortcuts: [
      { keys: "tab", action: "toggle target (chat ↔ last agent)" },
      { keys: "⌃tab / ⌃⇧tab", action: "cycle agents" },
      { keys: "enter", action: "send · open highlighted channel or card" },
      { keys: "↑ / ↓", action: "empty input: preview channels · walk prompts" },
      { keys: "⌥↑ / ⌥↓", action: "switch channels without leaving the box" },
      { keys: "← / →", action: "empty input: switch side-chat panes" },
      { keys: "esc", action: "back — side chat → card → navigator" },
      { keys: "/", action: "empty input: command palette" },
      { keys: "?", action: "empty input: this pane" },
    ],
  },
  {
    title: "global",
    shortcuts: [
      { keys: "⌘K", action: "command palette" },
      { keys: "⌘N", action: "new channel" },
      { keys: "⌘T", action: "new side chat in the open channel" },
      { keys: "⌘⇧T", action: "new tab (sub-channel)" },
      { keys: "⌘[ / ⌘]", action: "previous / next tab" },
      { keys: "⌘⇧D", action: "switch to standard ui" },
    ],
  },
];

/**
 * Keyboard-shortcuts pane for developer mode. The shell keeps the UI free
 * of key hints; this overlay (opened with `?` in an empty composer or via
 * the palette's "keyboard shortcuts" entry) is the single reference.
 */
export function DevShortcutsOverlay({ onClose }: { onClose: () => void }) {
  const panelRef = React.useRef<HTMLDivElement | null>(null);

  React.useEffect(() => {
    panelRef.current?.focus();
  }, []);

  return (
    <div className="absolute inset-0 z-50 flex items-start justify-center bg-background/60 pt-[10vh] font-mono">
      <div aria-hidden className="absolute inset-0" onClick={onClose} />
      <div
        ref={panelRef}
        className="relative flex max-h-[70vh] w-[560px] flex-col border border-border bg-background shadow-lg outline-none"
        data-testid="dev-mode-shortcuts-overlay"
        onKeyDown={(event) => {
          if (event.key === "Escape" || event.key === "Enter") {
            event.preventDefault();
            onClose();
          }
        }}
        role="dialog"
        aria-label="Keyboard shortcuts"
        tabIndex={-1}
      >
        <div className="flex shrink-0 items-baseline justify-between border-b border-border/60 px-3 py-2 text-sm">
          <span className="text-foreground">keyboard shortcuts</span>
          <button
            className="cursor-pointer text-xs text-muted-foreground hover:text-foreground"
            onClick={onClose}
            type="button"
          >
            close
          </button>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto px-3 py-2">
          {SHORTCUT_GROUPS.map((group) => (
            <div key={group.title} className="pb-3">
              <div className="pb-1 text-xs uppercase tracking-wide text-muted-foreground/50">
                {group.title}
              </div>
              {group.shortcuts.map((shortcut) => (
                <div
                  key={shortcut.keys}
                  className="flex items-baseline gap-3 py-0.5 text-sm"
                >
                  <span className="w-24 shrink-0 text-foreground">
                    {shortcut.keys}
                  </span>
                  <span className="min-w-0 text-muted-foreground">
                    {shortcut.action}
                  </span>
                </div>
              ))}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
