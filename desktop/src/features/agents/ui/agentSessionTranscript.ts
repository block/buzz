import type {
  ObserverEvent,
  PromptSection,
  ToolStatus,
  TranscriptItem,
} from "./agentSessionTypes";
import {
  findBuzzToolName,
  isGenericToolTitle,
  normalizeToolStatus,
} from "./agentSessionToolCatalog";
import { classifyTool } from "./agentSessionToolClassifier";
import { asRecord, asString } from "./agentSessionUtils";
import {
  describeTurnStarted,
  describeSessionResolved,
  extractBlockText,
  extractContentText,
  extractPromptText,
  extractToolArgs,
  extractToolIdentity,
  extractToolResult,
  parsePromptText,
  parseSystemPromptSections,
} from "./agentSessionTranscriptHelpers";

export { describeRawEvent } from "./agentSessionTranscriptHelpers";

export type TranscriptState = {
  items: TranscriptItem[];
  itemsById: Map<string, TranscriptItem>;
  activeMessageKey: Map<string, string>;
  sealedKeys: Set<string>;
  continuationSeq: number;
  latestSessionId: string | null;
};

export function createEmptyTranscriptState(): TranscriptState {
  return {
    items: [],
    itemsById: new Map(),
    activeMessageKey: new Map(),
    sealedKeys: new Set(),
    continuationSeq: 0,
    latestSessionId: null,
  };
}

/**
 * Mutable draft that collects changes during a single processTranscriptEvent
 * call. Replaces the previous pattern of nested closures capturing bare `let`
 * bindings — all mutation now targets this explicit object.
 */
type TranscriptDraft = {
  items: TranscriptItem[];
  itemsById: Map<string, TranscriptItem>;
  activeMessageKey: Map<string, string>;
  sealedKeys: Set<string>;
  continuationSeq: number;
  latestSessionId: string | null;
  changed: boolean;
};

function draftFrom(state: TranscriptState): TranscriptDraft {
  return {
    items: state.items,
    itemsById: state.itemsById,
    activeMessageKey: state.activeMessageKey,
    sealedKeys: state.sealedKeys,
    continuationSeq: state.continuationSeq,
    latestSessionId: state.latestSessionId,
    changed: false,
  };
}

/** Lazily copy items + itemsById on first mutation so callers get new refs. */
function ensureMutable(d: TranscriptDraft) {
  if (!d.changed) {
    d.items = [...d.items];
    d.itemsById = new Map(d.itemsById);
    d.changed = true;
  }
}

function replaceItem(d: TranscriptDraft, id: string, updated: TranscriptItem) {
  ensureMutable(d);
  const idx = d.items.findIndex((it) => it.id === id);
  if (idx !== -1) {
    d.items[idx] = updated;
  }
  d.itemsById.set(id, updated);
}

function pushItem(d: TranscriptDraft, item: TranscriptItem) {
  ensureMutable(d);
  d.items.push(item);
  d.itemsById.set(item.id, item);
}

function sealOpenMessages(d: TranscriptDraft) {
  let copied = false;
  for (const [, currentKey] of d.activeMessageKey) {
    if (!d.sealedKeys.has(currentKey)) {
      if (!copied) {
        d.sealedKeys = new Set(d.sealedKeys);
        copied = true;
      }
      d.sealedKeys.add(currentKey);
    }
  }
}

type TranscriptItemContext = {
  channelId: string | null;
  turnId: string | null;
  sessionId: string | null;
};

function upsertMessage(
  d: TranscriptDraft,
  id: string,
  role: "assistant" | "user",
  title: string,
  text: string,
  timestamp: string,
  ctx: TranscriptItemContext,
  authorPubkey: string | null = null,
  acpSource?: string,
) {
  const currentKey = d.activeMessageKey.get(id);

  if (currentKey && !d.sealedKeys.has(currentKey)) {
    const existing = d.itemsById.get(currentKey);
    if (existing?.type === "message") {
      replaceItem(d, currentKey, {
        ...existing,
        text: existing.text + text,
        channelId: ctx.channelId,
        turnId: ctx.turnId ?? existing.turnId,
        sessionId: ctx.sessionId ?? existing.sessionId,
        authorPubkey: authorPubkey ?? existing.authorPubkey,
        acpSource: acpSource ?? existing.acpSource,
      });
      return;
    }
  }

  d.continuationSeq += 1;
  const newKey = currentKey ? `${id}:c${d.continuationSeq}` : id;
  pushItem(d, {
    id: newKey,
    type: "message",
    renderClass: "message",
    role,
    title,
    text,
    timestamp,
    channelId: ctx.channelId,
    turnId: ctx.turnId,
    sessionId: ctx.sessionId,
    authorPubkey,
    acpSource,
  });
  d.activeMessageKey = new Map(d.activeMessageKey);
  d.activeMessageKey.set(id, newKey);
}

function upsertTextItem(
  d: TranscriptDraft,
  id: string,
  type: "thought" | "lifecycle",
  title: string,
  text: string,
  timestamp: string,
  ctx: TranscriptItemContext,
  acpSource?: string,
) {
  const existing = d.itemsById.get(id);
  if (existing && existing.type === type) {
    replaceItem(d, id, {
      ...existing,
      text: existing.text + text,
      channelId: ctx.channelId,
      turnId: ctx.turnId ?? existing.turnId,
      sessionId: ctx.sessionId ?? existing.sessionId,
      acpSource: acpSource ?? existing.acpSource,
    });
    return;
  }
  sealOpenMessages(d);
  if (type === "thought") {
    pushItem(d, {
      id,
      type: "thought",
      renderClass: "thought",
      title,
      text,
      timestamp,
      channelId: ctx.channelId,
      turnId: ctx.turnId,
      sessionId: ctx.sessionId,
      acpSource,
    });
    return;
  }

  pushItem(d, {
    id,
    type: "lifecycle",
    renderClass: title.toLowerCase().includes("error") ? "error" : "status",
    title,
    text,
    timestamp,
    channelId: ctx.channelId,
    turnId: ctx.turnId,
    sessionId: ctx.sessionId,
    acpSource,
  });
}

function upsertPlan(
  d: TranscriptDraft,
  id: string,
  title: string,
  text: string,
  timestamp: string,
  ctx: TranscriptItemContext,
  acpSource?: string,
  updateMarkerId?: string,
) {
  const existing = d.itemsById.get(id);
  if (existing?.type === "plan") {
    const changed = existing.text !== text;
    replaceItem(d, id, {
      ...existing,
      text,
      channelId: ctx.channelId,
      turnId: ctx.turnId ?? existing.turnId,
      sessionId: ctx.sessionId ?? existing.sessionId,
      acpSource: acpSource ?? existing.acpSource,
    });
    if (changed) {
      pushItem(d, {
        id: updateMarkerId ?? `${id}:update:${timestamp}`,
        type: "plan",
        renderClass: "plan",
        title: "Plan updated",
        text: summarizePlanUpdate(text),
        timestamp,
        isUpdate: true,
        targetId: id,
        channelId: ctx.channelId,
        turnId: ctx.turnId,
        sessionId: ctx.sessionId,
        acpSource,
      });
    }
    return;
  }
  sealOpenMessages(d);
  pushItem(d, {
    id,
    type: "plan",
    renderClass: "plan",
    title,
    text,
    timestamp,
    channelId: ctx.channelId,
    turnId: ctx.turnId,
    sessionId: ctx.sessionId,
    acpSource,
  });
}

function summarizePlanUpdate(text: string) {
  const taskMatches = [...text.matchAll(/\[[ xX]\]/g)];
  if (taskMatches.length > 0) {
    const completed = taskMatches.filter((match) =>
      match[0].toLowerCase().includes("x"),
    ).length;
    return `${completed}/${taskMatches.length} complete`;
  }

  const stepCount = text
    .split(/\r?\n/)
    .filter((line) => /^\s*(?:[-*]|\d+[.)])\s+\S/.test(line)).length;
  return stepCount > 0 ? `${stepCount} step${stepCount === 1 ? "" : "s"}` : "";
}

function upsertMetadata(
  d: TranscriptDraft,
  id: string,
  title: string,
  sections: PromptSection[],
  timestamp: string,
  ctx: TranscriptItemContext,
  acpSource?: string,
) {
  const existing = d.itemsById.get(id);
  if (existing?.type === "metadata") {
    replaceItem(d, id, {
      ...existing,
      sections,
      channelId: ctx.channelId,
      turnId: ctx.turnId ?? existing.turnId,
      sessionId: ctx.sessionId ?? existing.sessionId,
      acpSource: acpSource ?? existing.acpSource,
    });
    return;
  }
  sealOpenMessages(d);
  pushItem(d, {
    id,
    type: "metadata",
    renderClass: "raw-rail",
    title,
    sections,
    timestamp,
    channelId: ctx.channelId,
    turnId: ctx.turnId,
    sessionId: ctx.sessionId,
    acpSource,
  });
}

function isTerminalToolStatus(status: ToolStatus) {
  return status === "completed" || status === "failed";
}

function mergeToolStatus(existing: ToolStatus, next: ToolStatus): ToolStatus {
  if (isTerminalToolStatus(existing) && !isTerminalToolStatus(next)) {
    return existing;
  }

  return next;
}

function upsertTool(
  d: TranscriptDraft,
  id: string,
  title: string,
  toolName: string,
  buzzToolName: string | null,
  status: ToolStatus,
  args: Record<string, unknown>,
  result: string,
  isError: boolean,
  timestamp: string,
  ctx: TranscriptItemContext,
  acpSource?: string,
) {
  const existing = d.itemsById.get(id);
  const canonicalBuzzToolName =
    buzzToolName ?? findBuzzToolName(toolName, true);
  if (existing?.type === "tool") {
    const updatedTitle = !isGenericToolTitle(title) ? title : existing.title;
    let updatedToolName = existing.toolName;
    let updatedBuzzToolName = existing.buzzToolName;
    if (canonicalBuzzToolName) {
      updatedBuzzToolName = canonicalBuzzToolName;
      updatedToolName = canonicalBuzzToolName;
    } else if (!existing.buzzToolName && !isGenericToolTitle(toolName)) {
      updatedToolName = toolName;
    }
    const mergedStatus = mergeToolStatus(existing.status, status);
    const updatedArgs = Object.keys(args).length > 0 ? args : existing.args;
    const updatedResult = result || existing.result;
    const updatedIsError = isError || existing.isError;
    const descriptor = classifyTool({
      title: updatedTitle,
      toolName: updatedToolName,
      buzzToolName: updatedBuzzToolName,
      args: updatedArgs,
      result: updatedResult,
      isError: updatedIsError || mergedStatus === "failed",
    });
    replaceItem(d, id, {
      ...existing,
      renderClass: descriptor.renderClass,
      descriptor,
      title: updatedTitle,
      toolName: updatedToolName,
      buzzToolName: updatedBuzzToolName,
      status: mergedStatus,
      args: updatedArgs,
      result: updatedResult,
      isError: updatedIsError,
      completedAt:
        isTerminalToolStatus(mergedStatus) && existing.completedAt == null
          ? timestamp
          : existing.completedAt,
      channelId: ctx.channelId,
      turnId: ctx.turnId ?? existing.turnId,
      sessionId: ctx.sessionId ?? existing.sessionId,
      acpSource: acpSource ?? existing.acpSource,
    });
    return;
  }
  const resolvedToolName = canonicalBuzzToolName ?? toolName;
  const descriptor = classifyTool({
    title,
    toolName: resolvedToolName,
    buzzToolName: canonicalBuzzToolName,
    args,
    result,
    isError: isError || status === "failed",
  });
  sealOpenMessages(d);
  pushItem(d, {
    id,
    type: "tool",
    renderClass: descriptor.renderClass,
    descriptor,
    title,
    toolName: resolvedToolName,
    buzzToolName: canonicalBuzzToolName,
    status,
    args,
    result,
    isError,
    timestamp,
    startedAt: timestamp,
    completedAt: null,
    channelId: ctx.channelId,
    turnId: ctx.turnId,
    sessionId: ctx.sessionId,
    acpSource,
  });
}

export function processTranscriptEvent(
  state: TranscriptState,
  event: ObserverEvent,
): TranscriptState {
  const d = draftFrom(state);

  if (event.sessionId && event.sessionId !== d.latestSessionId) {
    d.latestSessionId = event.sessionId;
  }

  const channelId = event.channelId ?? null;
  const ch = channelId ?? "global";
  const ctx: TranscriptItemContext = {
    channelId,
    turnId: event.turnId,
    sessionId: event.sessionId ?? d.latestSessionId,
  };

  if (event.kind === "turn_started") {
    upsertTextItem(
      d,
      `turn:${ch}:${event.turnId ?? event.seq}`,
      "lifecycle",
      "Turn started",
      describeTurnStarted(event.payload),
      event.timestamp,
      ctx,
      event.kind,
    );
  } else if (event.kind === "session_resolved") {
    upsertTextItem(
      d,
      `session:${ch}:${event.turnId ?? event.seq}`,
      "lifecycle",
      "Session ready",
      describeSessionResolved(event.payload),
      event.timestamp,
      ctx,
      event.kind,
    );
  } else if (event.kind === "acp_parse_error") {
    upsertTextItem(
      d,
      `parse-error:${ch}:${event.seq}`,
      "lifecycle",
      "Wire parse error",
      extractBlockText(event.payload),
      event.timestamp,
      ctx,
      event.kind,
    );
  } else if (event.kind === "turn_error" || event.kind === "agent_panic") {
    const payload = asRecord(event.payload);
    const outcome = asString(payload.outcome) ?? "error";
    const error = asString(payload.error) ?? "Unknown error";
    const title =
      event.kind === "agent_panic" ? "Agent error (crash)" : "Turn error";
    upsertTextItem(
      d,
      `${event.kind}:${ch}:${event.turnId ?? event.seq}`,
      "lifecycle",
      title,
      `${outcome}: ${error}`,
      event.timestamp,
      ctx,
      event.kind,
    );
  } else if (event.kind === "acp_read" || event.kind === "acp_write") {
    const payload = asRecord(event.payload);
    const method = asString(payload.method);

    if (event.kind === "acp_write" && method === "session/prompt") {
      const promptText = extractPromptText(payload);
      if (promptText) {
        const parsedPrompt = parsePromptText(promptText);
        if (parsedPrompt.userText) {
          upsertMessage(
            d,
            `prompt:${ch}:${event.turnId ?? event.seq}`,
            "user",
            parsedPrompt.userTitle,
            parsedPrompt.userText,
            event.timestamp,
            ctx,
            parsedPrompt.userPubkey,
            "session/prompt:user",
          );
        }
        if (parsedPrompt.sections.length > 0) {
          upsertMetadata(
            d,
            `prompt-context:${ch}:${event.turnId ?? event.seq}`,
            "Prompt context",
            parsedPrompt.sections,
            event.timestamp,
            ctx,
            "session/prompt:context",
          );
        }
      }
    } else if (event.kind === "acp_write" && method === "session/new") {
      // The base + persona prompts ride session/new's systemPrompt, framed by
      // the harness as [Base]/[System]. Surface them as one "System prompt" item
      // keyed per channel-session — the frame carries no session id (it predates
      // session creation), and session/new fires once per channel-session, so a
      // re-created session correctly replaces the prior item.
      const params = asRecord(payload.params);
      const systemPrompt = asString(params.systemPrompt);
      if (systemPrompt) {
        const sections = parseSystemPromptSections(systemPrompt);
        if (sections.length > 0) {
          upsertMetadata(
            d,
            `system-prompt:${ch}`,
            "System prompt",
            sections,
            event.timestamp,
            ctx,
          );
        }
      }
    } else if (
      event.kind === "acp_write" &&
      method === "_goose/unstable/session/steer"
    ) {
      const promptText = extractPromptText(payload);
      if (promptText) {
        const parsedPrompt = parsePromptText(promptText);
        if (parsedPrompt.userText) {
          upsertMessage(
            d,
            `steer:${ch}:${event.turnId ?? event.seq}`,
            "user",
            parsedPrompt.userTitle,
            parsedPrompt.userText,
            event.timestamp,
            ctx,
            parsedPrompt.userPubkey,
          );
        }
        if (parsedPrompt.sections.length > 0) {
          upsertMetadata(
            d,
            `steer-context:${ch}:${event.turnId ?? event.seq}`,
            "Prompt context",
            parsedPrompt.sections,
            event.timestamp,
            ctx,
          );
        }
      }
    } else if (event.kind === "acp_read" && method === "session/update") {
      const params = asRecord(payload.params);
      const update = asRecord(params.update);
      const updateType = asString(update.sessionUpdate) ?? "unknown";
      const turnKey = event.turnId ?? event.sessionId ?? "unknown";
      const messageId = asString(update.messageId);

      if (updateType === "agent_message_chunk") {
        upsertMessage(
          d,
          `assistant:${ch}:${messageId ?? turnKey}`,
          "assistant",
          "Assistant",
          extractContentText(update.content),
          event.timestamp,
          ctx,
          null,
          updateType,
        );
      } else if (updateType === "user_message_chunk") {
        // Suppress user_message_chunk echo when a steer already rendered
        // the user message for this turn (Goose echoes steered content back).
        const steerKey = `steer:${ch}:${event.turnId ?? event.seq}`;
        const authorPubkey = asString(update.authorPubkey);
        if (!d.itemsById.has(steerKey)) {
          upsertMessage(
            d,
            `user:${ch}:${messageId ?? turnKey}`,
            "user",
            "User",
            extractContentText(update.content),
            event.timestamp,
            ctx,
            authorPubkey,
            updateType,
          );
        }
      } else if (updateType === "agent_thought_chunk") {
        upsertTextItem(
          d,
          `thinking:${ch}:${messageId ?? turnKey}`,
          "thought",
          "Thinking",
          extractContentText(update.content),
          event.timestamp,
          ctx,
          updateType,
        );
      } else if (updateType === "tool_call") {
        const toolId = asString(update.toolCallId) ?? `tool:${event.seq}`;
        const identity = extractToolIdentity(update);
        upsertTool(
          d,
          `tool:${ch}:${toolId}`,
          identity.title,
          identity.toolName,
          identity.buzzToolName,
          normalizeToolStatus(asString(update.status) ?? "executing"),
          extractToolArgs(update),
          extractToolResult(update),
          false,
          event.timestamp,
          ctx,
          updateType,
        );
      } else if (updateType === "tool_call_update") {
        const toolId = asString(update.toolCallId) ?? `tool:${event.seq}`;
        const status = normalizeToolStatus(
          asString(update.status) ?? "completed",
        );
        const identity = extractToolIdentity(update);
        upsertTool(
          d,
          `tool:${ch}:${toolId}`,
          identity.title,
          identity.toolName,
          identity.buzzToolName,
          status,
          extractToolArgs(update),
          extractToolResult(update),
          status === "failed",
          event.timestamp,
          ctx,
          updateType,
        );
      } else if (updateType === "plan") {
        upsertPlan(
          d,
          `plan:${ch}:${turnKey}`,
          "Plan",
          extractContentText(update.content) || JSON.stringify(update, null, 2),
          event.timestamp,
          ctx,
          updateType,
          `plan-update:${ch}:${turnKey}:${event.seq}`,
        );
      }
    }
  }

  if (!d.changed && d.latestSessionId === state.latestSessionId) {
    return state;
  }

  return {
    items: d.items,
    itemsById: d.itemsById,
    activeMessageKey: d.activeMessageKey,
    sealedKeys: d.sealedKeys,
    continuationSeq: d.continuationSeq,
    latestSessionId: d.latestSessionId,
  };
}

export function buildTranscriptState(events: ObserverEvent[]): TranscriptState {
  let state = createEmptyTranscriptState();
  for (const event of events) {
    state = processTranscriptEvent(state, event);
  }
  return state;
}

export function buildTranscript(events: ObserverEvent[]): TranscriptItem[] {
  return buildTranscriptState(events).items;
}
