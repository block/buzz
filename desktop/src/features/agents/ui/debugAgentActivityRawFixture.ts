import type { ObserverEvent, TranscriptItem } from "./agentSessionTypes";
import { buildTranscript } from "./agentSessionTranscript";
import { DEBUG_AGENT_ACTIVITY_FIXTURE } from "./debugAgentActivityFixture";

function rawContent(text: string) {
  return [
    {
      type: "content",
      content: {
        type: "text",
        text,
      },
    },
  ];
}

function sessionUpdatePayload(update: Record<string, unknown>) {
  return {
    jsonrpc: "2.0",
    method: "session/update",
    params: {
      sessionId: update.sessionId,
      update,
    },
  };
}

function promptPayloadForMessage(
  item: Extract<TranscriptItem, { type: "message" }>,
) {
  const promptContext = DEBUG_AGENT_ACTIVITY_FIXTURE.find(
    (candidate): candidate is Extract<TranscriptItem, { type: "metadata" }> =>
      candidate.type === "metadata" &&
      candidate.acpSource === "session/prompt:context" &&
      candidate.turnId === item.turnId,
  );
  const sections = promptContext?.sections ?? [
    {
      title: "Buzz event",
      body: `From: ${item.title}\nContent: ${item.text}`,
    },
  ];

  return {
    jsonrpc: "2.0",
    method: "session/prompt",
    params: {
      sessionId: item.sessionId,
      prompt: sections.map((section) => ({
        type: "text",
        text: `[${section.title}]\n${section.body}`,
      })),
    },
  };
}

function payloadForItem(item: TranscriptItem): unknown {
  if (item.type === "message") {
    if (item.acpSource === "session/prompt:user") {
      return promptPayloadForMessage(item);
    }

    return sessionUpdatePayload({
      sessionId: item.sessionId,
      sessionUpdate:
        item.role === "assistant"
          ? "agent_message_chunk"
          : "user_message_chunk",
      messageId: item.id,
      content: rawContent(item.text),
    });
  }

  if (item.type === "thought") {
    return sessionUpdatePayload({
      sessionId: item.sessionId,
      sessionUpdate: "agent_thought_chunk",
      messageId: item.id,
      content: rawContent(item.text),
    });
  }

  if (item.type === "plan") {
    return sessionUpdatePayload({
      sessionId: item.sessionId,
      sessionUpdate: "plan",
      content: rawContent(item.text),
    });
  }

  if (item.type === "metadata") {
    if (item.acpSource === "raw_json_rpc") {
      const rawBody = item.sections[0]?.body;
      if (rawBody) {
        try {
          return JSON.parse(rawBody);
        } catch {
          return { type: "raw_json_rpc", body: rawBody };
        }
      }
    }
    return {
      jsonrpc: "2.0",
      method: "session/prompt",
      params: {
        context: item.sections,
      },
    };
  }

  if (item.type === "tool") {
    return sessionUpdatePayload({
      sessionId: item.sessionId,
      sessionUpdate: "tool_call_update",
      toolCallId: item.id,
      title: item.title,
      toolName: item.toolName,
      status: item.status,
      rawInput: item.args,
      content: rawContent(item.result),
      rawOutput: {
        isError: item.isError,
      },
    });
  }

  if (item.acpSource === "turn_started") {
    return {
      type: "turn_started",
      triggeringEventIds: ["debug-trigger-event"],
    };
  }

  if (item.acpSource === "session_resolved") {
    return {
      sessionId: item.sessionId,
      isNewSession: false,
    };
  }

  if (item.acpSource === "permission_request") {
    return {
      jsonrpc: "2.0",
      method: "session/request_permission",
      params: {
        toolCallId: item.id,
        title: item.title,
        options: [
          { optionId: "allow_once", kind: "allow_once", name: "Allow" },
          { optionId: "reject_once", kind: "reject_once", name: "Reject" },
        ],
      },
    };
  }

  if (item.acpSource === "turn_error") {
    return {
      outcome: "recovered",
      error: item.text,
    };
  }

  return {
    type: item.acpSource ?? item.renderClass,
    title: item.title,
    text: item.text,
  };
}

function kindForItem(item: TranscriptItem) {
  if (item.acpSource === "turn_started") return "turn_started";
  if (item.acpSource === "session_resolved") return "session_resolved";
  if (item.type === "message" && item.acpSource === "session/prompt:user") {
    return "acp_write";
  }
  if (item.acpSource === "permission_request") return "acp_read";
  if (item.acpSource === "turn_error") return "turn_error";
  if (item.acpSource === "raw_json_rpc") return "raw_json_rpc";
  if (item.type === "metadata" && item.acpSource === "session/prompt:context") {
    return "acp_write";
  }
  return "acp_read";
}

export const DEBUG_AGENT_ACTIVITY_RAW_EVENTS: ObserverEvent[] =
  DEBUG_AGENT_ACTIVITY_FIXTURE.map((item, index) => ({
    seq: index + 1,
    timestamp: item.timestamp,
    kind: kindForItem(item),
    agentIndex: 0,
    channelId: item.channelId ?? null,
    sessionId: item.sessionId ?? null,
    turnId: item.turnId ?? null,
    payload: payloadForItem(item),
  }));

export const DEBUG_AGENT_ACTIVITY_TRANSCRIPT = buildTranscript(
  DEBUG_AGENT_ACTIVITY_RAW_EVENTS,
);
