import * as React from "react";
import { Channel, invoke, isTauri } from "@tauri-apps/api/core";

import type { TerminalFrame } from "./terminalRenderer";

export type TerminalAttachRequest = {
  sessionId?: string;
  channelId: string;
  channelName: string;
  threadId?: string;
  npub: string;
  relayUrl: string;
  columns: number;
  rows: number;
  pixelWidth: number;
  pixelHeight: number;
  /** Optional env injected at PTY spawn (well-formed keys only). */
  extraEnv?: Record<string, string>;
  /** Optional PTY working directory. Omit for login-shell default (usually home). */
  cwd?: string;
  /** Optional shell path. Omit for system $SHELL resolution. */
  shell?: string;
  /** Optional program to spawn instead of the login shell (e.g. herdr). */
  program?: string;
  /** Arguments for `program` when set. */
  args?: string[];
  /** Optional scrollback depth. Omit for 10_000. */
  scrollback?: number;
  /** Web only: force this session to a plain shell on the desktop even when its Buzz Term setting is herdr (handoff sessions). */
  remoteHost?: "shell";
  /** Web only: re-drive this desktop terminal (it outlived a page refresh / dropped link) instead of launching a new one. */
  reattachId?: string;
};

export type TerminalViewport = TerminalFrame["viewport"];

export type TerminalFrameMessage = TerminalFrame & {
  subscriptionId: string;
  sequence: number;
  bracketedPaste: boolean;
  focusReporting: boolean;
  mouseReporting?: boolean;
};

export type TerminalMessage =
  | { type: "frame"; payload: TerminalFrameMessage }
  | { type: "title"; payload: string }
  | { type: "resetTitle"; payload?: never }
  | { type: "bell"; payload?: never }
  | { type: "exit"; payload?: never };

type AttachResponse = {
  sessionId: string;
  subscriptionId: string;
  viewport: TerminalViewport;
};

export type TerminalDelivery = {
  frame: TerminalFrameMessage;
  acknowledge: () => Promise<void>;
};

/** What the Bootstrap/Substrate need from a terminal session; the desktop (Tauri) and the web (remote listener) both provide it. */
export interface TerminalConnectionApi {
  readonly sessionId: string;
  readonly subscriptionId: string;
  readonly viewport: TerminalViewport;
  input(data: string): Promise<void>;
  resize(columns: number, rows: number, pixelWidth: number, pixelHeight: number): Promise<TerminalViewport>;
  viewportReady(viewport: TerminalViewport): Promise<void>;
  scroll(lines: number): Promise<void>;
  mouse(event: {
    button: "left" | "middle" | "right" | "wheelUp" | "wheelDown";
    action: "press" | "release" | "move";
    column: number;
    row: number;
    shift?: boolean;
    meta?: boolean;
    ctrl?: boolean;
  }): Promise<boolean>;
  focus(focused: boolean): Promise<void>;
  detach(): Promise<void>;
  close(): Promise<void>;
}

/**
 * Where sessions come from when this is not the Tauri desktop. The web app registers a backend that talks to the member's
 * desktop over the direct listener; the desktop never registers one.
 */
export type TerminalBackend = {
  attach(
    request: TerminalAttachRequest,
    onMessage: (message: Exclude<TerminalMessage, { type: "frame" }>) => void,
    onFrame: (delivery: TerminalDelivery) => void,
  ): Promise<TerminalConnectionApi>;
  /** Web only: terminals still running on the desktop from an earlier connection, to rebuild the tabs. */
  listLive?(): Promise<Array<{ id: string; title?: string | null }>>;
  /** An attach (or later op) failed; the web uses this to show why and close the panel. */
  onError?(error: unknown): void;
};

let backend: TerminalBackend | null = null;
const backendListeners = new Set<() => void>();
export function setTerminalBackend(next: TerminalBackend | null) {
  if (backend === next) return;
  backend = next;
  for (const l of backendListeners) l();
}
export function getTerminalBackend(): TerminalBackend | null {
  return backend;
}
export function useTerminalBackend(): TerminalBackend | null {
  return React.useSyncExternalStore(
    (l) => {
      backendListeners.add(l);
      return () => backendListeners.delete(l);
    },
    getTerminalBackend,
    () => null,
  );
}

export class TerminalConnection implements TerminalConnectionApi {
  readonly sessionId: string;
  readonly subscriptionId: string;
  readonly viewport: TerminalViewport;

  private constructor(response: AttachResponse) {
    this.sessionId = response.sessionId;
    this.subscriptionId = response.subscriptionId;
    this.viewport = response.viewport;
  }

  static async attach(
    request: TerminalAttachRequest,
    onMessage: (message: Exclude<TerminalMessage, { type: "frame" }>) => void,
    onFrame: (delivery: TerminalDelivery) => void,
  ): Promise<TerminalConnectionApi> {
    if (!isTauri()) {
      if (backend) return backend.attach(request, onMessage, onFrame);
      throw new Error("terminal sessions require Buzz Desktop");
    }

    let connection: TerminalConnection | null = null;
    const pending: TerminalMessage[] = [];
    const deliver = (message: TerminalMessage) => {
      if (!connection) {
        pending.push(message);
        return;
      }
      if (message.type !== "frame") {
        onMessage(message);
        return;
      }
      const frame = message.payload;
      if (frame.subscriptionId !== connection.subscriptionId) return;
      onFrame({
        frame,
        acknowledge: () =>
          connection?.acknowledge(frame.sequence) ?? Promise.resolve(),
      });
    };
    const channel = new Channel<TerminalMessage>(deliver);
    const response = await invoke<AttachResponse>("terminal_attach", {
      request,
      onFrame: channel,
    });
    connection = new TerminalConnection(response);
    for (const message of pending) deliver(message);
    return connection;
  }

  input(data: string): Promise<void> {
    return invoke("terminal_input", { sessionId: this.sessionId, data });
  }

  async resize(
    columns: number,
    rows: number,
    pixelWidth: number,
    pixelHeight: number,
  ): Promise<TerminalViewport> {
    return invoke("terminal_resize", {
      sessionId: this.sessionId,
      columns,
      rows,
      pixelWidth,
      pixelHeight,
    });
  }

  viewportReady(viewport: TerminalViewport): Promise<void> {
    return invoke("terminal_viewport_ready", {
      sessionId: this.sessionId,
      subscriptionId: this.subscriptionId,
      viewport,
    });
  }

  /**
   * Scroll the viewport by whole cells.
   *
   * `lines` keeps the DOM's sign: negative is the direction that scrolls a
   * page toward the top of the document. The backend owns the conversion to
   * the engine's opposite convention, so this layer carries no policy about
   * which way history is.
   */
  scroll(lines: number): Promise<void> {
    return invoke("terminal_scroll", { sessionId: this.sessionId, lines });
  }

  mouse(event: {
    button: "left" | "middle" | "right" | "wheelUp" | "wheelDown";
    action: "press" | "release" | "move";
    column: number;
    row: number;
    shift?: boolean;
    meta?: boolean;
    ctrl?: boolean;
  }): Promise<boolean> {
    return invoke("terminal_mouse", {
      request: { sessionId: this.sessionId, ...event },
    });
  }

  focus(focused: boolean): Promise<void> {
    return invoke("terminal_focus", { sessionId: this.sessionId, focused });
  }

  detach(): Promise<void> {
    return invoke("terminal_detach", {
      sessionId: this.sessionId,
      subscriptionId: this.subscriptionId,
    });
  }

  close(): Promise<void> {
    return invoke("terminal_close", { sessionId: this.sessionId });
  }

  private acknowledge(sequence: number): Promise<void> {
    return invoke("terminal_ack", {
      sessionId: this.sessionId,
      subscriptionId: this.subscriptionId,
      sequence,
    });
  }
}
