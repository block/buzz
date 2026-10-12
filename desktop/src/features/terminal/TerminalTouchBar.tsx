import * as React from "react";

import { cn } from "@/shared/lib/cn";
import { TOUCH_KEYS } from "./touchTerminal";

type Props = {
  ctrl: boolean;
  alt: boolean;
  onToggleCtrl: () => void;
  onToggleAlt: () => void;
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
export function TerminalTouchBar({ ctrl, alt, onToggleCtrl, onToggleAlt, onKey, onPaste, keyboardOpen, onToggleKeyboard }: Props) {
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
      {key("Tab", TOUCH_KEYS.tab, "tk-tab")}
      {mod("Ctrl", ctrl, onToggleCtrl, "tk-ctrl")}
      {mod("Alt", alt, onToggleAlt, "tk-alt")}
      {key("←", TOUCH_KEYS.left, "tk-left", true)}
      {key("↓", TOUCH_KEYS.down, "tk-down", true)}
      {key("↑", TOUCH_KEYS.up, "tk-up", true)}
      {key("→", TOUCH_KEYS.right, "tk-right", true)}
      {key("^C", TOUCH_KEYS.ctrlC, "tk-ctrlc")}
      <button type="button" className="buzz-terminal-touch-key" data-testid="tk-paste" onPointerDown={press(onPaste)} {...common}>
        Paste
      </button>
      <button type="button" className="buzz-terminal-touch-key" data-testid="tk-keyboard" aria-label={keyboardOpen ? "Hide keyboard" : "Show keyboard"} onPointerDown={press(onToggleKeyboard)} {...common}>
        {keyboardOpen ? "⌄" : "⌨"}
      </button>
    </div>
  );
}
