import { invoke } from "@tauri-apps/api/core";
import { signRelayEvent } from "@/shared/api/tauri";
import type { RelayEvent, SendChannelMessageResult } from "@/shared/api/types";
import { KIND_STREAM_MESSAGE } from "@/shared/constants/kinds";

export type MessageSendTransport = "websocket" | "http";

export type MessageSendDiagnosticFields = {
  channelId?: string;
  eventId?: string;
  waitMs?: number;
  gateRemainingMs?: number;
  connectionState?: string;
  outcome?: string;
};

export type MessageSendTrace = {
  operationId: string;
  mark: (stage: string, fields?: MessageSendDiagnosticFields) => void;
  measure: <T>(
    stage: string,
    operation: () => Promise<T>,
    fields?: MessageSendDiagnosticFields,
  ) => Promise<T>;
  finish: <T>(operation: () => Promise<T>, eventId?: string) => Promise<T>;
  finishSuccess: (eventId?: string) => void;
  finishFailure: (error: unknown) => void;
};

export type SendTracedStreamMessageOptions = {
  trace: MessageSendTrace;
  channelId: string;
  content: string;
  mentionPubkeys: string[];
  extraTags: string[][];
  ensureConnected: () => Promise<void>;
  connectionState: () => string;
  publishEvent: (
    event: RelayEvent,
    timeoutMessage: string,
    sendErrorMessage: string,
    trace: MessageSendTrace,
  ) => Promise<RelayEvent>;
};

function classifySendFailure(error: unknown): string {
  const message = error instanceof Error ? error.message.toLowerCase() : "";
  if (message.includes("timed out") || message.includes("timeout")) {
    return "timeout";
  }
  if (message.includes("rate-limit")) return "rate_limited";
  if (message.includes("reject")) return "relay_rejected";
  if (message.includes("connect") || message.includes("socket")) {
    return "connection_error";
  }
  return "unknown_error";
}

export function createMessageSendTrace({
  channelId,
  transport,
}: {
  channelId: string;
  transport: MessageSendTransport;
}): MessageSendTrace {
  const operationId = crypto.randomUUID();
  const startedAt = performance.now();
  let finished = false;
  let writeQueue = Promise.resolve();

  const mark = (stage: string, fields: MessageSendDiagnosticFields = {}) => {
    const entry = {
      operationId,
      stage,
      transport,
      channelId,
      elapsedMs: Math.max(0, Math.round(performance.now() - startedAt)),
      ...fields,
    };
    writeQueue = writeQueue
      .then(() => invoke("append_message_send_diagnostic", { entry }))
      .then(() => undefined)
      .catch(() => {
        // Diagnostics must never affect message delivery.
      });
  };

  mark("client_send_started");

  return {
    operationId,
    mark,
    async measure(stage, operation, fields) {
      const stageStartedAt = performance.now();
      mark(`${stage}_started`, fields);
      try {
        const result = await operation();
        mark(`${stage}_finished`, {
          ...fields,
          waitMs: Math.round(performance.now() - stageStartedAt),
          outcome: "accepted",
        });
        return result;
      } catch (error) {
        mark(`${stage}_finished`, {
          ...fields,
          waitMs: Math.round(performance.now() - stageStartedAt),
          outcome: "failed",
        });
        throw error;
      }
    },
    async finish(operation, eventId) {
      try {
        const result = await operation();
        this.finishSuccess(eventId);
        return result;
      } catch (error) {
        this.finishFailure(error);
        throw error;
      }
    },
    finishSuccess(eventId) {
      if (finished) return;
      finished = true;
      mark("client_send_finished", { eventId, outcome: "accepted" });
    },
    finishFailure(error) {
      if (finished) return;
      finished = true;
      mark("client_send_finished", { outcome: classifySendFailure(error) });
    },
  };
}

export function measureMessageSendStage<T>(
  trace: MessageSendTrace | undefined,
  stage: string,
  operation: () => Promise<T>,
  fields?: MessageSendDiagnosticFields,
): Promise<T> {
  return trace?.measure(stage, operation, fields) ?? operation();
}

export async function sendTracedStreamMessage({
  trace,
  channelId,
  content,
  mentionPubkeys,
  extraTags,
  ensureConnected,
  connectionState,
  publishEvent,
}: SendTracedStreamMessageOptions): Promise<RelayEvent> {
  try {
    await trace.measure("connection_wait", ensureConnected, {
      connectionState: connectionState(),
    });
    const tags = [
      ["h", channelId],
      ...mentionPubkeys.map((pubkey) => ["p", pubkey]),
      ...extraTags,
    ];
    const event = await trace.measure("event_sign", () =>
      signRelayEvent({
        kind: KIND_STREAM_MESSAGE,
        content: content.trim(),
        tags,
      }),
    );
    trace.mark("event_signed", { eventId: event.id });
    return await trace.finish(
      () =>
        publishEvent(
          event,
          "Timed out while sending the message.",
          "Failed to send the message.",
          trace,
        ),
      event.id,
    );
  } catch (error) {
    trace.finishFailure(error);
    throw error;
  }
}

type RawSendChannelMessageResult = {
  event_id: string;
  parent_event_id: string | null;
  root_event_id: string | null;
  depth: number;
  created_at: number;
};

export async function sendChannelMessageWithDiagnostics(
  channelId: string,
  content: string,
  parentEventId: string | null,
  mediaTags: string[][],
  mentionPubkeys: string[],
  emojiTags: string[][],
  mentionTags: string[][],
  linkPreviewTags: string[][],
  diagnosticId: string,
): Promise<SendChannelMessageResult> {
  const response = await invoke<RawSendChannelMessageResult>(
    "send_channel_message",
    {
      channelId,
      content,
      parentEventId,
      mediaTags,
      emojiTags,
      mentionTags,
      linkPreviewTags,
      mentionPubkeys,
      kind: null,
      diagnosticId,
    },
  );
  return {
    eventId: response.event_id,
    parentEventId: response.parent_event_id,
    rootEventId: response.root_event_id,
    depth: response.depth,
    createdAt: response.created_at,
  };
}
