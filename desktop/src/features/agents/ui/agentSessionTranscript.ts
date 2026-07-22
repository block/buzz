import type {
  AgentActivityDescriptor,
  AgentActivityRenderClass,
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
import { asRecord, asString, titleCase } from "./agentSessionUtils";
import {
  describePermissionOutcome,
  describePermissionRequest,
  jsonRpcId,
} from "./agentSessionPermission";
import {
  describeTurnStarted,
  describeSessionResolved,
  extractBlockText,
  extractContentText,
  extractPlanText,
  extractPromptText,
  extractTriggeringEventIds,
  extractToolArgs,
  extractToolIdentity,
  extractToolResult,
  parsePromptText,
  parseSystemPromptSections,
} from "./agentSessionTranscriptHelpers";
import { friendlyTurnErrorCopy } from "../lib/friendlyAgentLastError";

export { describeRawEvent } from "./agentSessionTranscriptHelpers";

export type TranscriptState = {
  items: TranscriptItem[];
  itemsById: Map<string, TranscriptItem>;
  activeMessageKey: Map<string, string>;
  sealedKeys: Set<string>;
  triggeringEventIdsByTurn: Map<string, string[]>;
  /**
   * Maps conversation-scoped JSON-RPC request id → { itemId, optionNames }.
   * Populated when a `session/request_permission` request is ingested so the
   * matching response (which carries the same JSON-RPC id, no `method`) can
   * correlate and append the outcome to the lifecycle item. ACP processes
   * commonly allocate request ids from small local counters, so the canonical
   * channel/thread scope is part of the key.
   */
  pendingPermissions: Map<
    string,
    { itemId: string; optionNames: Map<string, string> }
  >;
  continuationSeq: number;
  /** Latest resolved session per channel/thread conversation identity. */
  latestSessionIdByConversation: Map<string, string>;
};

export function createEmptyTranscriptState(): TranscriptState {
  return {
    items: [],
    itemsById: new Map(),
    activeMessageKey: new Map(),
    sealedKeys: new Set(),
    triggeringEventIdsByTurn: new Map(),
    pendingPermissions: new Map(),
    continuationSeq: 0,
    latestSessionIdByConversation: new Map(),
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
  triggeringEventIdsByTurn: Map<string, string[]>;
  pendingPermissions: Map<
    string,
    { itemId: string; optionNames: Map<string, string> }
  >;
  continuationSeq: number;
  latestSessionIdByConversation: Map<string, string>;
  changed: boolean;
};

function draftFrom(state: TranscriptState): TranscriptDraft {
  return {
    items: state.items,
    itemsById: state.itemsById,
    activeMessageKey: state.activeMessageKey,
    sealedKeys: state.sealedKeys,
    triggeringEventIdsByTurn: state.triggeringEventIdsByTurn,
    pendingPermissions: state.pendingPermissions,
    continuationSeq: state.continuationSeq,
    latestSessionIdByConversation: state.latestSessionIdByConversation,
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

function sealOpenMessages(d: TranscriptDraft, ctx: TranscriptItemContext) {
  let copied = false;
  for (const [, currentKey] of d.activeMessageKey) {
    const current = d.itemsById.get(currentKey);
    if (
      !current ||
      current.channelId !== ctx.channelId ||
      (current.threadRoot ?? null) !== ctx.threadRoot
    ) {
      continue;
    }
    if (!d.sealedKeys.has(currentKey)) {
      if (!copied) {
        d.sealedKeys = new Set(d.sealedKeys);
        copied = true;
      }
      d.sealedKeys.add(currentKey);
    }
  }
}

function turnMapKey(channelKey: string, turnKey: string | number | null) {
  return `${channelKey}:${turnKey ?? "unknown"}`;
}

function rememberTriggeringEventIds(
  d: TranscriptDraft,
  channelKey: string,
  turnKey: string | number | null,
  ids: string[],
) {
  if (ids.length === 0) return;
  d.triggeringEventIdsByTurn = new Map(d.triggeringEventIdsByTurn);
  d.triggeringEventIdsByTurn.set(turnMapKey(channelKey, turnKey), ids);
}

function getSingleTriggeringEventId(
  d: TranscriptDraft,
  channelKey: string,
  turnKey: string | number | null,
) {
  const ids = d.triggeringEventIdsByTurn.get(turnMapKey(channelKey, turnKey));
  return ids?.length === 1 ? maybeNostrEventId(ids[0]) : null;
}

function maybeNostrEventId(id: string | null | undefined) {
  return id && /^[0-9a-fA-F]{64}$/.test(id) ? id : null;
}

function stringifyPayload(value: unknown) {
  try {
    return JSON.stringify(value, null, 2) ?? String(value);
  } catch {
    return String(value);
  }
}

function describeFreeformStatus(payload: Record<string, unknown>) {
  const statusType = asString(payload.type) ?? asString(payload.status);
  const title =
    asString(payload.title) ?? (statusType ? titleCase(statusType) : null);
  const text = asString(payload.text) ?? asString(payload.message);
  if (!title || !text) return null;
  return { statusType: statusType ?? title.toLowerCase(), title, text };
}

function rawPayloadTitle(payload: unknown) {
  const record = asRecord(payload);
  return asString(record.method) ?? asString(record.type) ?? "raw_json_rpc";
}

type TranscriptItemContext = {
  channelId: string | null;
  threadRoot: string | null;
  turnId: string | null;
  sessionId: string | null;
};

function transcriptConversationKey(
  channelId: string | null,
  threadRoot: string | null,
): string {
  return `${channelId ?? "global"}\u0000${threadRoot ?? ""}`;
}

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
  messageId: string | null = null,
) {
  const currentKey = d.activeMessageKey.get(id);

  if (currentKey && !d.sealedKeys.has(currentKey)) {
    const existing = d.itemsById.get(currentKey);
    if (existing?.type === "message") {
      replaceItem(d, currentKey, {
        ...existing,
        text: existing.text + text,
        channelId: ctx.channelId,
        threadRoot: ctx.threadRoot ?? existing.threadRoot,
        turnId: ctx.turnId ?? existing.turnId,
        sessionId: ctx.sessionId ?? existing.sessionId,
        authorPubkey: authorPubkey ?? existing.authorPubkey,
        acpSource: acpSource ?? existing.acpSource,
        messageId: messageId ?? existing.messageId,
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
    messageId,
    channelId: ctx.channelId,
    threadRoot: ctx.threadRoot,
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
      text:
        type === "lifecycle"
          ? joinLifecycleText(existing.text, text)
          : existing.text + text,
      channelId: ctx.channelId,
      threadRoot: ctx.threadRoot ?? existing.threadRoot,
      turnId: ctx.turnId ?? existing.turnId,
      sessionId: ctx.sessionId ?? existing.sessionId,
      acpSource: acpSource ?? existing.acpSource,
    });
    return;
  }
  sealOpenMessages(d, ctx);
  if (type === "thought") {
    pushItem(d, {
      id,
      type: "thought",
      renderClass: "thought",
      title,
      text,
      timestamp,
      channelId: ctx.channelId,
      threadRoot: ctx.threadRoot,
      turnId: ctx.turnId,
      sessionId: ctx.sessionId,
      acpSource,
    });
    return;
  }

  upsertLifecycleItem(
    d,
    id,
    title.toLowerCase().includes("error") ? "error" : "status",
    title,
    text,
    timestamp,
    ctx,
    acpSource,
  );
}

function joinLifecycleText(existing: string, next: string) {
  if (!existing) return next;
  if (!next) return existing;
  return `${existing}\n${next}`;
}

function upsertLifecycleItem(
  d: TranscriptDraft,
  id: string,
  renderClass: Extract<
    AgentActivityRenderClass,
    "status" | "permission" | "error"
  >,
  title: string,
  text: string,
  timestamp: string,
  ctx: TranscriptItemContext,
  acpSource?: string,
  descriptor?: AgentActivityDescriptor,
) {
  const existing = d.itemsById.get(id);
  if (existing?.type === "lifecycle") {
    replaceItem(d, id, {
      ...existing,
      renderClass,
      title,
      text: joinLifecycleText(existing.text, text),
      descriptor: descriptor ?? existing.descriptor,
      channelId: ctx.channelId,
      threadRoot: ctx.threadRoot ?? existing.threadRoot,
      turnId: ctx.turnId ?? existing.turnId,
      sessionId: ctx.sessionId ?? existing.sessionId,
      acpSource: acpSource ?? existing.acpSource,
    });
    return;
  }

  sealOpenMessages(d, ctx);
  pushItem(d, {
    id,
    type: "lifecycle",
    renderClass,
    title,
    text,
    timestamp,
    descriptor,
    channelId: ctx.channelId,
    threadRoot: ctx.threadRoot,
    turnId: ctx.turnId,
    sessionId: ctx.sessionId,
    acpSource,
  });
}

// Like upsertLifecycleItem but REPLACES the text on update instead of
// appending. Used for coalescing fields (e.g. usage_update) where only the
// latest value is meaningful — repeated updates must not accumulate.
function replaceLifecycleItem(
  d: TranscriptDraft,
  id: string,
  renderClass: Extract<
    AgentActivityRenderClass,
    "status" | "permission" | "error"
  >,
  title: string,
  text: string,
  timestamp: string,
  ctx: TranscriptItemContext,
  acpSource?: string,
) {
  const existing = d.itemsById.get(id);
  if (existing?.type === "lifecycle") {
    replaceItem(d, id, {
      ...existing,
      renderClass,
      title,
      text,
      channelId: ctx.channelId,
      threadRoot: ctx.threadRoot ?? existing.threadRoot,
      turnId: ctx.turnId ?? existing.turnId,
      sessionId: ctx.sessionId ?? existing.sessionId,
      acpSource: acpSource ?? existing.acpSource,
    });
    return;
  }

  sealOpenMessages(d, ctx);
  pushItem(d, {
    id,
    type: "lifecycle",
    renderClass,
    title,
    text,
    timestamp,
    channelId: ctx.channelId,
    threadRoot: ctx.threadRoot,
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
      threadRoot: ctx.threadRoot ?? existing.threadRoot,
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
        threadRoot: ctx.threadRoot,
        turnId: ctx.turnId,
        sessionId: ctx.sessionId,
        acpSource,
      });
    }
    return;
  }
  sealOpenMessages(d, ctx);
  pushItem(d, {
    id,
    type: "plan",
    renderClass: "plan",
    title,
    text,
    timestamp,
    channelId: ctx.channelId,
    threadRoot: ctx.threadRoot,
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
      threadRoot: ctx.threadRoot ?? existing.threadRoot,
      turnId: ctx.turnId ?? existing.turnId,
      sessionId: ctx.sessionId ?? existing.sessionId,
      acpSource: acpSource ?? existing.acpSource,
    });
    return;
  }
  sealOpenMessages(d, ctx);
  pushItem(d, {
    id,
    type: "metadata",
    renderClass: "raw-rail",
    title,
    sections,
    timestamp,
    channelId: ctx.channelId,
    threadRoot: ctx.threadRoot,
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
      threadRoot: ctx.threadRoot ?? existing.threadRoot,
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
  sealOpenMessages(d, ctx);
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
    completedAt: isTerminalToolStatus(status) ? timestamp : null,
    channelId: ctx.channelId,
    threadRoot: ctx.threadRoot,
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

  const channelId = event.channelId ?? null;
  const threadRoot = event.threadRoot ?? null;
  const conversationKey = transcriptConversationKey(channelId, threadRoot);
  const latestSessionId =
    d.latestSessionIdByConversation.get(conversationKey) ?? null;
  if (event.sessionId && event.sessionId !== latestSessionId) {
    d.latestSessionIdByConversation = new Map(d.latestSessionIdByConversation);
    d.latestSessionIdByConversation.set(conversationKey, event.sessionId);
  }
  const ch = channelId ?? "global";
  // ACP message/tool IDs are session-local for several adapters. Namespace
  // every transcript identity by the canonical conversation so sequential
  // A/B/A turns in one channel cannot merge equal IDs across thread sessions.
  // Keep the legacy key shape exactly unchanged when threadRoot is absent.
  const scopeKey = threadRoot ? `${ch}:thread:${threadRoot}` : ch;
  const ctx: TranscriptItemContext = {
    channelId,
    threadRoot,
    turnId: event.turnId,
    sessionId:
      event.sessionId ??
      d.latestSessionIdByConversation.get(conversationKey) ??
      null,
  };

  if (event.kind === "raw_json_rpc") {
    upsertMetadata(
      d,
      `raw-json-rpc:${scopeKey}:${event.seq}`,
      "Raw ACP payload",
      [
        {
          title: rawPayloadTitle(event.payload),
          body: stringifyPayload(event.payload),
        },
      ],
      event.timestamp,
      ctx,
      event.kind,
    );
  } else if (event.kind === "turn_started") {
    rememberTriggeringEventIds(
      d,
      scopeKey,
      event.turnId ?? event.seq,
      extractTriggeringEventIds(event.payload),
    );
    upsertTextItem(
      d,
      `turn:${scopeKey}:${event.turnId ?? event.seq}`,
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
      `session:${scopeKey}:${event.turnId ?? event.seq}`,
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
      `parse-error:${scopeKey}:${event.seq}`,
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
    const displayError = friendlyTurnErrorCopy(error, payload.code);
    const title =
      event.kind === "agent_panic" ? "Agent error (crash)" : "Turn error";
    upsertTextItem(
      d,
      `${event.kind}:${scopeKey}:${event.turnId ?? event.seq}`,
      "lifecycle",
      title,
      `${outcome}: ${displayError}`,
      event.timestamp,
      ctx,
      event.kind,
    );
  } else if (event.kind === "acp_read" || event.kind === "acp_write") {
    const payload = asRecord(event.payload);
    const method = asString(payload.method);

    if (method === "session/request_permission") {
      const request = describePermissionRequest(payload);
      const itemId = `permission:${scopeKey}:${event.turnId ?? event.seq}`;
      upsertLifecycleItem(
        d,
        itemId,
        "permission",
        "Permission requested",
        request.text,
        event.timestamp,
        ctx,
        "permission_request",
        request.descriptor,
      );
      // Index by JSON-RPC id so the response (acp_write with result.outcome,
      // no method) can correlate by id rather than by turn/seq.
      const requestId = jsonRpcId(payload.id);
      if (requestId) {
        d.pendingPermissions = new Map(d.pendingPermissions);
        d.pendingPermissions.set(`${scopeKey}\u0000${requestId}`, {
          itemId,
          optionNames: request.optionNames,
        });
      }
    } else if (event.kind === "acp_write" && !method) {
      // Permission response: {"id": <same as request>, "result": {"outcome": {...}}}
      const responseId = jsonRpcId(payload.id);
      const result = asRecord(asRecord(payload.result).outcome);
      const outcomeKind = asString(result.outcome);
      const pendingKey = responseId ? `${scopeKey}\u0000${responseId}` : null;
      const pending = pendingKey ? d.pendingPermissions.get(pendingKey) : null;
      if (pending && outcomeKind && responseId) {
        const optionId = asString(result.optionId) ?? null;
        const outcomeText = describePermissionOutcome(
          outcomeKind,
          optionId,
          pending.optionNames,
        );
        const existing = d.itemsById.get(pending.itemId);
        if (existing?.type === "lifecycle") {
          replaceItem(d, pending.itemId, {
            ...existing,
            outcome: outcomeText,
          });
          // Remove from pending map — the outcome is now recorded.
          d.pendingPermissions = new Map(d.pendingPermissions);
          if (pendingKey) {
            d.pendingPermissions.delete(pendingKey);
          }
        }
      }
    } else if (event.kind === "acp_write" && method === "session/prompt") {
      const promptText = extractPromptText(payload);
      if (promptText) {
        const parsedPrompt = parsePromptText(promptText);
        if (parsedPrompt.userText) {
          upsertMessage(
            d,
            `prompt:${scopeKey}:${event.turnId ?? event.seq}`,
            "user",
            parsedPrompt.userTitle,
            parsedPrompt.userText,
            event.timestamp,
            ctx,
            parsedPrompt.userPubkey,
            "session/prompt:user",
            parsedPrompt.userEventId ??
              getSingleTriggeringEventId(
                d,
                scopeKey,
                event.turnId ?? event.seq,
              ),
          );
        }
        if (parsedPrompt.sections.length > 0) {
          upsertMetadata(
            d,
            `prompt-context:${scopeKey}:${event.turnId ?? event.seq}`,
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
      // the harness as [Base]/[System]/[Agent Memory — core]/[Channel Canvas].
      // claude-agent-acp uses _meta.systemPrompt.append instead; both paths
      // produce the same standalone card (turnId: null, acpSource "session/new");
      // the bare field takes precedence when both are present.
      const params = asRecord(payload.params);
      const metaPrompt = asString(
        asRecord(asRecord(params._meta).systemPrompt).append,
      );
      const systemPrompt = asString(params.systemPrompt) ?? metaPrompt;
      if (systemPrompt) {
        const sections = parseSystemPromptSections(systemPrompt);
        if (sections.length > 0) {
          upsertMetadata(
            d,
            `system-prompt:${scopeKey}:${event.seq}:${event.timestamp}`,
            "System prompt",
            sections,
            event.timestamp,
            { ...ctx, turnId: null },
            "session/new",
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
            `steer:${scopeKey}:${event.turnId ?? event.seq}`,
            "user",
            parsedPrompt.userTitle,
            parsedPrompt.userText,
            event.timestamp,
            ctx,
            parsedPrompt.userPubkey,
            "session/steer:user",
            parsedPrompt.userEventId,
          );
        }
        if (parsedPrompt.sections.length > 0) {
          upsertMetadata(
            d,
            `steer-context:${scopeKey}:${event.turnId ?? event.seq}`,
            "Prompt context",
            parsedPrompt.sections,
            event.timestamp,
            ctx,
            "session/steer:context",
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
          `assistant:${scopeKey}:${messageId ?? turnKey}`,
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
        const steerKey = `steer:${scopeKey}:${event.turnId ?? event.seq}`;
        const authorPubkey = asString(update.authorPubkey);
        if (!d.itemsById.has(steerKey)) {
          const channelMessageId = maybeNostrEventId(messageId);
          upsertMessage(
            d,
            `user:${scopeKey}:${messageId ?? turnKey}`,
            "user",
            "User",
            extractContentText(update.content),
            event.timestamp,
            ctx,
            authorPubkey,
            updateType,
            channelMessageId,
          );
        }
      } else if (updateType === "agent_thought_chunk") {
        upsertTextItem(
          d,
          `thinking:${scopeKey}:${messageId ?? turnKey}`,
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
          `tool:${scopeKey}:${toolId}`,
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
          `tool:${scopeKey}:${toolId}`,
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
          `plan:${scopeKey}:${turnKey}`,
          "Plan",
          extractPlanText(update),
          event.timestamp,
          ctx,
          updateType,
          `plan-update:${scopeKey}:${turnKey}:${event.seq}`,
        );
      } else if (updateType === "current_mode_update") {
        const mode = asString(update.currentModeId) ?? "";
        if (mode) {
          upsertLifecycleItem(
            d,
            `mode:${scopeKey}:${turnKey}`,
            "status",
            "Mode",
            mode,
            event.timestamp,
            ctx,
            updateType,
          );
        }
      } else if (updateType === "usage_update") {
        const used = typeof update.used === "number" ? update.used : null;
        const size = typeof update.size === "number" ? update.size : null;
        if (used !== null && size !== null) {
          const costRecord = asRecord(update.cost);
          const costAmount =
            typeof costRecord.amount === "number" ? costRecord.amount : null;
          const costCurrency = asString(costRecord.currency);
          const costStr =
            costAmount !== null && costCurrency
              ? ` ($${costAmount.toFixed(4)} ${costCurrency})`
              : "";
          replaceLifecycleItem(
            d,
            `usage:${scopeKey}:${turnKey}`,
            "status",
            "Usage",
            `Tokens: ${used}/${size}${costStr}`,
            event.timestamp,
            ctx,
            updateType,
          );
        }
      } else if (updateType === "available_commands_update") {
        const cmds = Array.isArray(update.availableCommands)
          ? update.availableCommands
          : [];
        upsertLifecycleItem(
          d,
          `commands:${scopeKey}:${turnKey}`,
          "status",
          "Commands",
          `Commands available: ${cmds.length}`,
          event.timestamp,
          ctx,
          updateType,
        );
      } else if (updateType === "config_option_update") {
        const opts = Array.isArray(update.configOptions)
          ? (update.configOptions as Array<Record<string, unknown>>)
          : [];
        const optText = opts
          .map((o) => {
            const name = asString(o.name) ?? asString(o.id) ?? "?";
            const val =
              asString(o.currentValue) ??
              (typeof o.value === "boolean" ? String(o.value) : null) ??
              "";
            return val ? `${name} = ${val}` : name;
          })
          .join(", ");
        if (optText) {
          upsertLifecycleItem(
            d,
            `config:${scopeKey}:${turnKey}`,
            "status",
            "Config",
            optText,
            event.timestamp,
            ctx,
            updateType,
          );
        }
      } else {
        // Free-form observer status records are not part of the ACP session/update
        // union. Surface only explicit title/text payloads; leave all other
        // unknown frames out of the feed instead of guessing at semantics.
        const status = describeFreeformStatus(payload);
        if (status) {
          upsertLifecycleItem(
            d,
            `status:${scopeKey}:${event.turnId ?? event.seq}:${status.statusType}`,
            "status",
            status.title,
            status.text,
            event.timestamp,
            ctx,
            status.statusType,
          );
        }
      }
    } else {
      // Free-form observer status records are not part of the ACP JSON-RPC
      // method set. Surface only explicit title/text payloads; leave all other
      // unknown frames out of the feed instead of guessing at semantics.
      const status = describeFreeformStatus(payload);
      if (status) {
        upsertLifecycleItem(
          d,
          `status:${scopeKey}:${event.turnId ?? event.seq}:${status.statusType}`,
          "status",
          status.title,
          status.text,
          event.timestamp,
          ctx,
          status.statusType,
        );
      }
    }
  }

  if (
    !d.changed &&
    d.latestSessionIdByConversation === state.latestSessionIdByConversation
  ) {
    return state;
  }

  return {
    items: d.items,
    itemsById: d.itemsById,
    activeMessageKey: d.activeMessageKey,
    sealedKeys: d.sealedKeys,
    triggeringEventIdsByTurn: d.triggeringEventIdsByTurn,
    pendingPermissions: d.pendingPermissions,
    continuationSeq: d.continuationSeq,
    latestSessionIdByConversation: d.latestSessionIdByConversation,
  };
}

export function buildTranscriptState(
  events: readonly ObserverEvent[],
): TranscriptState {
  let state = createEmptyTranscriptState();
  for (const event of events) {
    state = processTranscriptEvent(state, event);
  }
  return state;
}

export function buildTranscript(
  events: readonly ObserverEvent[],
): TranscriptItem[] {
  return buildTranscriptState(events).items;
}
