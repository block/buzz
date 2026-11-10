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
};

export type TerminalViewport = TerminalFrame["viewport"];

export type TerminalFrameMessage = TerminalFrame & {
  subscriptionId: string;
  sequence: number;
  bracketedPaste: boolean;
  focusReporting: boolean;
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

export class TerminalConnection {
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
  ): Promise<TerminalConnection> {
    if (!isTauri()) throw new Error("terminal sessions require Buzz Desktop");

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
