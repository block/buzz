import * as React from "react";
import { Terminal } from "@xterm/xterm";
import type { ITheme } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import "@xterm/xterm/css/xterm.css";

/**
 * Read the semantic terminal CSS vars and build an xterm.js ITheme.
 * Falls back to a neutral dark palette if vars are not yet applied.
 */
function getTerminalTheme(): ITheme {
  const style = getComputedStyle(document.documentElement);

  const hslToHex = (hslValue: string): string | undefined => {
    if (!hslValue?.trim()) return undefined;
    // Parse "H S% L%" format from CSS var
    const parts = hslValue.trim().split(/\s+/);
    if (parts.length < 3) return undefined;
    const h = Number.parseFloat(parts[0]);
    const s = Number.parseFloat(parts[1]) / 100;
    const l = Number.parseFloat(parts[2]) / 100;

    const a = s * Math.min(l, 1 - l);
    const f = (n: number) => {
      const k = (n + h / 30) % 12;
      const color = l - a * Math.max(Math.min(k - 3, 9 - k, 1), -1);
      return Math.round(255 * color)
        .toString(16)
        .padStart(2, "0");
    };
    return `#${f(0)}${f(8)}${f(4)}`;
  };

  const bg =
    hslToHex(style.getPropertyValue("--terminal-background")) ?? "#1a1b26";
  const fg =
    hslToHex(style.getPropertyValue("--terminal-foreground")) ?? "#c0caf5";

  const isDark = document.documentElement.classList.contains("dark");

  return {
    background: bg,
    foreground: fg,
    cursor: fg,
    selectionBackground: isDark ? "#33467c" : "#2f3549",
    black: bg,
    red: isDark ? "#f7768e" : "#ff757f",
    green: isDark ? "#9ece6a" : "#c3e88d",
    yellow: isDark ? "#e0af68" : "#ffc777",
    blue: isDark ? "#7aa2f7" : "#82aaff",
    magenta: isDark ? "#bb9af7" : "#c099ff",
    cyan: isDark ? "#7dcfff" : "#86e1fc",
    white: fg,
    brightBlack: isDark ? "#414868" : "#545c7e",
    brightRed: isDark ? "#f7768e" : "#ff757f",
    brightGreen: isDark ? "#9ece6a" : "#c3e88d",
    brightYellow: isDark ? "#e0af68" : "#ffc777",
    brightBlue: isDark ? "#7aa2f7" : "#82aaff",
    brightMagenta: isDark ? "#bb9af7" : "#c099ff",
    brightCyan: isDark ? "#7dcfff" : "#86e1fc",
    brightWhite: fg,
  };
}

type TerminalDataPayload = {
  sessionId: string;
  data: string;
};

type TerminalExitPayload = {
  sessionId: string;
  exitCode: number;
};

type TerminalOpenOutput = {
  sessionId: string;
  created: boolean;
  initialData: string | null;
};

type TerminalInstanceProps = {
  channelId: string;
  isVisible: boolean;
};

/**
 * A single xterm.js terminal instance backed by a native PTY session.
 * Scoped to a channel — reattaches on re-render if the session already exists.
 */
export function TerminalInstance({
  channelId,
  isVisible,
}: TerminalInstanceProps) {
  const containerRef = React.useRef<HTMLDivElement>(null);
  const termRef = React.useRef<Terminal | null>(null);
  const fitAddonRef = React.useRef<FitAddon | null>(null);
  const sessionIdRef = React.useRef<string | null>(null);
  const unlistenDataRef = React.useRef<UnlistenFn | null>(null);
  const unlistenExitRef = React.useRef<UnlistenFn | null>(null);

  React.useEffect(() => {
    if (!containerRef.current || !isVisible) return;

    const term = new Terminal({
      cursorBlink: true,
      fontSize: 13,
      fontFamily: "'SF Mono', 'Fira Code', 'JetBrains Mono', Menlo, monospace",
      lineHeight: 1.3,
      scrollback: 10000,
      allowProposedApi: true,
      theme: getTerminalTheme(),
    });

    const fitAddon = new FitAddon();
    const webLinksAddon = new WebLinksAddon();

    term.loadAddon(fitAddon);
    term.loadAddon(webLinksAddon);
    term.open(containerRef.current);

    termRef.current = term;
    fitAddonRef.current = fitAddon;

    // Fit after a frame so the container has dimensions.
    requestAnimationFrame(() => {
      fitAddon.fit();
    });

    // Open PTY session.
    const cols = term.cols;
    const rows = term.rows;

    invoke<TerminalOpenOutput>("terminal_open_session", {
      input: { channelId, cols, rows },
    }).then((output) => {
      sessionIdRef.current = output.sessionId;

      // Write buffered output from a reattached session.
      if (output.initialData) {
        term.write(output.initialData);
      }

      // Listen for PTY data.
      listen<TerminalDataPayload>("terminal:data", (event) => {
        if (event.payload.sessionId === sessionIdRef.current) {
          term.write(event.payload.data);
        }
      }).then((unlisten) => {
        unlistenDataRef.current = unlisten;
      });

      // Listen for PTY exit.
      listen<TerminalExitPayload>("terminal:exit", (event) => {
        if (event.payload.sessionId === sessionIdRef.current) {
          term.write(
            `\r\n\x1b[90m[Process exited with code ${event.payload.exitCode}]\x1b[0m\r\n`,
          );
          sessionIdRef.current = null;
        }
      }).then((unlisten) => {
        unlistenExitRef.current = unlisten;
      });

      // Send keystrokes to PTY.
      term.onData((data) => {
        if (sessionIdRef.current) {
          invoke("terminal_write", {
            input: { sessionId: sessionIdRef.current, data },
          });
        }
      });
    });

    // Watch for theme class changes on <html> to sync xterm colors.
    const themeObserver = new MutationObserver(() => {
      term.options.theme = getTerminalTheme();
    });
    themeObserver.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["class"],
    });

    // Resize observer.
    const resizeObserver = new ResizeObserver(() => {
      if (fitAddonRef.current && isVisible) {
        fitAddonRef.current.fit();
        if (sessionIdRef.current && termRef.current) {
          invoke("terminal_resize", {
            input: {
              sessionId: sessionIdRef.current,
              cols: termRef.current.cols,
              rows: termRef.current.rows,
            },
          });
        }
      }
    });
    resizeObserver.observe(containerRef.current);

    return () => {
      themeObserver.disconnect();
      resizeObserver.disconnect();
      unlistenDataRef.current?.();
      unlistenExitRef.current?.();
      term.dispose();
      termRef.current = null;
      fitAddonRef.current = null;
    };
  }, [channelId, isVisible]);

  // Re-fit when visibility changes.
  React.useEffect(() => {
    if (isVisible && fitAddonRef.current) {
      requestAnimationFrame(() => {
        fitAddonRef.current?.fit();
        if (sessionIdRef.current && termRef.current) {
          invoke("terminal_resize", {
            input: {
              sessionId: sessionIdRef.current,
              cols: termRef.current.cols,
              rows: termRef.current.rows,
            },
          });
        }
      });
    }
  }, [isVisible]);

  return (
    <div
      ref={containerRef}
      className="h-full w-full"
      style={{ display: isVisible ? "block" : "none" }}
    />
  );
}
