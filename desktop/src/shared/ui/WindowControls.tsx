import * as React from "react";

import { cn } from "@/shared/lib/cn";

/**
 * Windows caption buttons for the undecorated main window.
 *
 * The window is created without native decorations on Windows (see
 * `lib.rs`), so the app draws minimize/maximize/close itself. Sizing follows
 * the Fluent caption metrics (46x32 hit target, 10px glyph) rather than the
 * app's rem scale: like the macOS traffic lights these must not grow or shrink
 * with Cmd +/- text zoom. Deliberate exception to the rem-first rule.
 */

const CAPTION_BUTTON_CLASS =
  "inline-flex h-[32px] w-[46px] items-center justify-center text-sidebar-foreground/75 transition-colors hover:bg-sidebar-accent hover:text-sidebar-accent-foreground";

function MinimizeGlyph() {
  return (
    <svg aria-hidden="true" height="10" viewBox="0 0 10 10" width="10">
      <path d="M0 5h10" fill="none" stroke="currentColor" strokeWidth="1" />
    </svg>
  );
}

function MaximizeGlyph({ maximized }: { maximized: boolean }) {
  if (maximized) {
    return (
      <svg aria-hidden="true" height="10" viewBox="0 0 10 10" width="10">
        <path
          d="M2.5 0.5h7v7M0.5 2.5h7v7h-7z"
          fill="none"
          stroke="currentColor"
          strokeWidth="1"
        />
      </svg>
    );
  }

  return (
    <svg aria-hidden="true" height="10" viewBox="0 0 10 10" width="10">
      <path
        d="M0.5 0.5h9v9h-9z"
        fill="none"
        stroke="currentColor"
        strokeWidth="1"
      />
    </svg>
  );
}

function CloseGlyph() {
  return (
    <svg aria-hidden="true" height="10" viewBox="0 0 10 10" width="10">
      <path
        d="M0 0l10 10M10 0L0 10"
        fill="none"
        stroke="currentColor"
        strokeWidth="1"
      />
    </svg>
  );
}

export function WindowControls() {
  const [maximized, setMaximized] = React.useState(false);
  const windowRef = React.useRef<{
    minimize: () => Promise<void>;
    toggleMaximize: () => Promise<void>;
    close: () => Promise<void>;
    isMaximized: () => Promise<boolean>;
    onResized: (handler: () => void) => Promise<() => void>;
  } | null>(null);

  React.useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;

    void (async () => {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      if (cancelled) {
        return;
      }

      const appWindow = getCurrentWindow();
      windowRef.current = appWindow;
      setMaximized(await appWindow.isMaximized());

      unlisten = await appWindow.onResized(() => {
        void appWindow.isMaximized().then(setMaximized);
      });
    })();

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  return (
    // Not a drag region: caption buttons must receive clicks, and the
    // surrounding chrome already carries `data-tauri-drag-region`.
    <div className="ml-auto flex items-center self-stretch">
      <button
        aria-label="Minimize"
        className={CAPTION_BUTTON_CLASS}
        onClick={() => void windowRef.current?.minimize()}
        type="button"
      >
        <MinimizeGlyph />
      </button>
      <button
        aria-label={maximized ? "Restore" : "Maximize"}
        className={CAPTION_BUTTON_CLASS}
        onClick={() => void windowRef.current?.toggleMaximize()}
        type="button"
      >
        <MaximizeGlyph maximized={maximized} />
      </button>
      <button
        aria-label="Close"
        className={cn(
          CAPTION_BUTTON_CLASS,
          "hover:bg-[#c42b1c] hover:text-white",
        )}
        onClick={() => void windowRef.current?.close()}
        type="button"
      >
        <CloseGlyph />
      </button>
    </div>
  );
}
