import { ChevronDown, Keyboard } from "lucide-react";
import * as React from "react";
import { createPortal } from "react-dom";

import { cn } from "@/shared/lib/cn";
import { TOUCH_KEYS } from "./touchTerminal";

type Props = {
  ctrl: boolean;
  alt: boolean;
  meta: boolean;
  onToggleCtrl: () => void;
  onToggleAlt: () => void;
  onToggleMeta: () => void;
  /** Select mode: one-finger drags select cells instead of scrolling. */
  selectMode: boolean;
  onToggleSelect: () => void;
  /** Send raw terminal input (Esc, Tab, arrows, Ctrl-C). */
  onKey: (sequence: string) => void;
  onPaste: () => void;
  keyboardOpen: boolean;
  onToggleKeyboard: () => void;
};

/**
 * Keys a phone keyboard doesn't have. Buttons act on pointer-down and never take focus, so the hidden input (and with it the
 * on-screen keyboard) stays up while you tap them; arrows repeat while held.
 */
export type TouchKeyGroupItem = { id: string; label: string; ariaLabel?: string; on?: boolean; action: () => void };

/**
 * Reusable key-bar group: one button that opens a floating row of options directly above it. Tapping an option runs it and
 * closes the row; tapping outside closes it too. Buttons act on pointer-down and never take focus (keyboard stays up).
 */
export function TouchKeyGroup({ label, ariaLabel, items, testId }: { label: string; ariaLabel: string; items: TouchKeyGroupItem[]; testId: string }) {
  const [open, setOpen] = React.useState(false);
  const ref = React.useRef<HTMLDivElement>(null);
  const rowRef = React.useRef<HTMLDivElement>(null);
  const [pos, setPos] = React.useState<{ left: number; bottom: number } | null>(null);
  React.useLayoutEffect(() => {
    if (!open || !ref.current) return;
    // The key bar scrolls horizontally (it clips anything absolutely positioned), so the row is portaled and fixed above the button.
    const r = ref.current.getBoundingClientRect();
    const rowW = rowRef.current?.offsetWidth ?? 0;
    setPos({ left: Math.max(4, Math.min(r.left, window.innerWidth - rowW - 4)), bottom: window.innerHeight - r.top + 6 });
  }, [open]);
  React.useEffect(() => {
    if (!open) return;
    const away = (e: PointerEvent) => {
      const t = e.target as Node;
      if (!ref.current?.contains(t) && !rowRef.current?.contains(t)) setOpen(false);
    };
    document.addEventListener("pointerdown", away, true);
    return () => document.removeEventListener("pointerdown", away, true);
  }, [open]);
  const anyOn = items.some((i) => i.on);
  return (
    <div ref={ref} className="buzz-terminal-touch-group" data-testid={testId}>
      <button
        type="button"
        className={cn("buzz-terminal-touch-key", (open || anyOn) && "buzz-terminal-touch-key-on")}
        aria-label={ariaLabel}
        aria-expanded={open}
        data-testid={`${testId}-toggle`}
        onPointerDown={(e) => {
          e.preventDefault();
          setOpen((o) => !o);
        }}
        onMouseDown={(e) => e.preventDefault()}
      >
        {label}
      </button>
      {open ? createPortal(
        <div
          ref={rowRef}
          className="buzz-terminal-touch-group-row"
          role="menu"
          data-testid={`${testId}-row`}
          style={{ left: pos?.left ?? 4, bottom: pos?.bottom ?? 60, visibility: pos ? "visible" : "hidden" }}
        >
          {items.map((it) => (
            <button
              key={it.id}
              type="button"
              role="menuitem"
              className={cn("buzz-terminal-touch-key", it.on && "buzz-terminal-touch-key-on")}
              aria-label={it.ariaLabel ?? it.label}
              aria-pressed={it.on}
              data-testid={`${testId}-${it.id}`}
              onPointerDown={(e) => {
                e.preventDefault();
                it.action();
                setOpen(false);
              }}
              onMouseDown={(e) => e.preventDefault()}
            >
              {it.label}
            </button>
          ))}
        </div>,
        document.body,
      ) : null}
    </div>
  );
}

export function TerminalTouchBar({ ctrl, alt, meta, onToggleCtrl, onToggleAlt, onToggleMeta, selectMode, onToggleSelect, onKey, onPaste, keyboardOpen, onToggleKeyboard }: Props) {
  const repeat = React.useRef<{ delay: number; every: number }>({ delay: 0, every: 0 });
  const stop = React.useCallback(() => {
    window.clearTimeout(repeat.current.delay);
    window.clearInterval(repeat.current.every);
  }, []);
  React.useEffect(() => stop, [stop]);

  const press =
    (action: () => void, repeating = false) =>
    (event: React.PointerEvent) => {
      event.preventDefault();
      stop();
      action();
      if (!repeating) return;
      repeat.current.delay = window.setTimeout(() => {
        repeat.current.every = window.setInterval(action, 70);
      }, 380);
    };
  const common = { onPointerUp: stop, onPointerCancel: stop, onPointerLeave: stop, onMouseDown: (e: React.MouseEvent) => e.preventDefault() };

  const key = (label: string, seq: string, testId: string, repeating = false) => (
    <button key={testId} type="button" className="buzz-terminal-touch-key" data-testid={testId} aria-label={label} onPointerDown={press(() => onKey(seq), repeating)} {...common}>
      {label}
    </button>
  );
  const mod = (label: string, on: boolean, toggle: () => void, testId: string) => (
    <button
      key={testId}
      type="button"
      className={cn("buzz-terminal-touch-key", on && "buzz-terminal-touch-key-on")}
      data-testid={testId}
      aria-pressed={on}
      onPointerDown={press(toggle)}
      {...common}
    >
      {label}
    </button>
  );

  return (
    <div className="buzz-terminal-touchbar" data-testid="terminal-touchbar" role="toolbar" aria-label="Terminal keys">
      {key("Esc", TOUCH_KEYS.esc, "tk-esc")}
      <TouchKeyGroup
        label="⌃⌥"
        ariaLabel="Modifiers"
        testId="tk-mods"
        items={[
          { id: "tab", label: "Tab", action: () => onKey(TOUCH_KEYS.tab) },
          { id: "ctrl", label: "Ctrl", ariaLabel: "Control (next key)", on: ctrl, action: onToggleCtrl },
          { id: "alt", label: "⌥ Alt", ariaLabel: "Option (next key)", on: alt, action: onToggleAlt },
          { id: "cmd", label: "⌘ Cmd", ariaLabel: "Command (next key)", on: meta, action: onToggleMeta },
        ]}
      />
      {mod("Select", selectMode, onToggleSelect, "tk-select")}
      <TouchKeyGroup
        label="↑↓"
        ariaLabel="Arrow keys"
        testId="tk-arrows"
        items={[
          { id: "left", label: "←", ariaLabel: "Left", action: () => onKey(TOUCH_KEYS.left) },
          { id: "down", label: "↓", ariaLabel: "Down", action: () => onKey(TOUCH_KEYS.down) },
          { id: "up", label: "↑", ariaLabel: "Up", action: () => onKey(TOUCH_KEYS.up) },
          { id: "right", label: "→", ariaLabel: "Right", action: () => onKey(TOUCH_KEYS.right) },
        ]}
      />
      {key("^C", TOUCH_KEYS.ctrlC, "tk-ctrlc")}
      <button type="button" className="buzz-terminal-touch-key" data-testid="tk-paste" onPointerDown={press(onPaste)} {...common}>
        Paste
      </button>
      <button
        type="button"
        className="buzz-terminal-touch-key buzz-terminal-touch-icon"
        data-testid="tk-keyboard"
        aria-label={keyboardOpen ? "Hide keyboard" : "Show keyboard"}
        // One action per tap, on pointerup (inside the user gesture, so iOS lets the hidden input focus and raise the keyboard).
        // Down/mousedown are swallowed so the button never takes focus (which would blur the input) and no click doubles it.
        onPointerDown={(e) => e.preventDefault()}
        onMouseDown={(e) => e.preventDefault()}
        onClick={(e) => e.preventDefault()}
        onPointerUp={(e) => {
          e.preventDefault();
          onToggleKeyboard();
        }}
      >
        {keyboardOpen ? <ChevronDown aria-hidden size={24} strokeWidth={2} /> : <Keyboard aria-hidden size={24} strokeWidth={2} />}
      </button>
    </div>
  );
}
