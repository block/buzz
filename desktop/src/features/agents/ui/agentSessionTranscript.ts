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
  describeTurnStarted,
  describeSessionResolved,
  extractBlockText,
  extractContentText,
  extractPlanText,
  extractPromptBlocks,
  extractTriggeringEventIds,
  extractToolArgs,
  extractToolIdentity,
  extractToolResult,
  parsePromptBlocks,
  parseSystemPromptSections,
} from "./agentSessionTranscriptHelpers";
import { friendlyTurnErrorCopy } from "../lib/friendlyAgentLastError";
import {
  describePermissionOutcome,
  describePermissionRequest,
  jsonRpcId,
  ownerResolutionFromPending,
  permissionIdentity,
  permissionTerminalObserverOutcomes,
  rawJsonRpcId,
} from "./agentSessionTranscriptPermissions";
import {
  createEmptyTranscriptState,
  createTranscriptDraft,
  type TranscriptDraft,
  type TranscriptState,
} from "./agentSessionTranscriptState";

export { describeRawEvent } from "./agentSessionTranscriptHelpers";
export {
  createEmptyTranscriptState,
  type TranscriptState,
} from "./agentSessionTranscriptState";
export { projectPermissionLedgerEvents } from "./agentSessionTranscriptPermissions";

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
      turnId: ctx.turnId ?? existing.turnId,
      sessionId: ctx.sessionId ?? existing.sessionId,
      acpSource: acpSource ?? existing.acpSource,
    });
    return;
  }

  sealOpenMessages(d);
  pushItem(d, {
    id,
    type: "lifecycle",
    renderClass,
    title,
    text,
    timestamp,
    descriptor,
    channelId: ctx.channelId,
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
      turnId: ctx.turnId ?? existing.turnId,
      sessionId: ctx.sessionId ?? existing.sessionId,
      acpSource: acpSource ?? existing.acpSource,
    });
    return;
  }

  sealOpenMessages(d);
  pushItem(d, {
    id,
    type: "lifecycle",
    renderClass,
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
    completedAt: isTerminalToolStatus(status) ? timestamp : null,
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
  const d = createTranscriptDraft(state);

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

  if (event.kind === "raw_json_rpc") {
    upsertMetadata(
      d,
      `raw-json-rpc:${ch}:${event.seq}`,
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
      ch,
      event.turnId ?? event.seq,
      extractTriggeringEventIds(event.payload),
    );
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
    const displayError = friendlyTurnErrorCopy(error, payload.code);
    const title =
      event.kind === "agent_panic" ? "Agent error (crash)" : "Turn error";
    upsertTextItem(
      d,
      `${event.kind}:${ch}:${event.turnId ?? event.seq}`,
      "lifecycle",
      title,
      `${outcome}: ${displayError}`,
      event.timestamp,
      ctx,
      event.kind,
    );
  } else if (event.kind === "permission_pending") {
    const payload = asRecord(event.payload);
    const requestId = jsonRpcId(payload.requestId);
    const resolution = ownerResolutionFromPending(payload, event);
    const pending =
      requestId && resolution
        ? d.pendingPermissions.get(
            permissionIdentity(
              channelId,
              resolution.sessionId,
              resolution.turnId,
              requestId,
            ),
          )
        : null;
    const existing = pending ? d.itemsById.get(pending.itemId) : null;
    // A request is visible before this lifecycle marker. Keep the card inert
    // unless the exact request, session, and turn all agree.
    if (
      pending &&
      resolution &&
      existing?.type === "lifecycle" &&
      existing.turnId === resolution.turnId &&
      existing.sessionId === resolution.sessionId
    ) {
      replaceItem(d, pending.itemId, {
        ...existing,
        pendingResolution: resolution,
      });
    }
  } else if (event.kind === "permission_ledger") {
    const payload = asRecord(event.payload);
    const request = asRecord(payload.request);
    const rawRequestId = rawJsonRpcId(payload.requestId);
    const requestId = jsonRpcId(rawRequestId);
    const sessionId = asString(payload.sessionId);
    const turnId = asString(payload.turnId);
    const digest = asString(payload.actionDigest);
    const options = Array.isArray(payload.options) ? payload.options : [];
    const lifecycleState = asRecord(payload.state);
    const stateKind = asString(lifecycleState.kind);
    if (
      Object.keys(request).length === 0 ||
      !requestId ||
      rawRequestId === null ||
      !sessionId ||
      !turnId ||
      !digest
    )
      return state;
    const description = describePermissionRequest(request);
    const canonicalId = permissionIdentity(
      channelId,
      sessionId,
      turnId,
      requestId,
    );
    const existing = d.itemsById.get(`permission:${canonicalId}`);
    const existingPermission = existing?.type === "lifecycle" ? existing : null;
    const pendingResolution =
      stateKind === "pending" && !existingPermission?.outcome
        ? {
            turnId,
            sessionId,
            requestId: rawRequestId,
            actionDigest: digest,
            options: options.flatMap((option) => {
              const value = asRecord(option);
              const optionId = asString(value.optionId);
              return optionId
                ? [{ optionId, label: asString(value.name) ?? optionId }]
                : [];
            }),
          }
        : undefined;
    const outcome =
      stateKind === "selected"
        ? describePermissionOutcome(
            "selected",
            asString(lifecycleState.reason) ?? null,
            description.optionNames,
          )
        : stateKind === "cancelled" || stateKind === "expired"
          ? "Cancelled"
          : stateKind === "delivery_unknown"
            ? "Delivery unknown (not replayed)"
            : stateKind === "abandoned" ||
                stateKind === "decision_consumed" ||
                stateKind === "delivery_attempted"
              ? "Unavailable (agent session ended)"
              : undefined;
    const item = {
      id: `permission:${canonicalId}`,
      type: "lifecycle",
      renderClass: "permission",
      title: "Permission request",
      text: description.text,
      timestamp: event.timestamp,
      channelId,
      turnId,
      sessionId,
      pendingResolution,
      outcome,
      acpSource: event.kind,
    } as const;
    if (existing?.type === "lifecycle") {
      // A stale fetched Pending row must never re-enable a terminal live card.
      replaceItem(
        d,
        item.id,
        existingPermission?.outcome && stateKind === "pending"
          ? existingPermission
          : item,
      );
    } else {
      pushItem(d, item);
    }
    if (pendingResolution) {
      d.pendingPermissions = new Map(d.pendingPermissions);
      d.pendingPermissions.set(canonicalId, {
        itemId: item.id,
        optionNames: description.optionNames,
      });
    }
  } else if (Object.hasOwn(permissionTerminalObserverOutcomes, event.kind)) {
    const payload = asRecord(event.payload);
    const requestId = jsonRpcId(payload.requestId);
    const sessionId = asString(payload.sessionId);
    const key =
      requestId && sessionId && event.turnId
        ? permissionIdentity(channelId, sessionId, event.turnId, requestId)
        : null;
    const pending = key ? d.pendingPermissions.get(key) : null;
    const existing = pending ? d.itemsById.get(pending.itemId) : null;
    if (key && pending && existing?.type === "lifecycle") {
      replaceItem(d, pending.itemId, {
        ...existing,
        outcome: permissionTerminalObserverOutcomes[event.kind],
        pendingResolution: undefined,
      });
      d.pendingPermissions = new Map(d.pendingPermissions);
      d.pendingPermissions.delete(key);
    }
  } else if (event.kind === "acp_read" || event.kind === "acp_write") {
    const payload = asRecord(event.payload);
    const method = asString(payload.method);

    if (method === "session/request_permission") {
      const request = describePermissionRequest(payload);
      const requestId = jsonRpcId(payload.id);
      const requestSessionId =
        asString(asRecord(payload.params).sessionId) ?? ctx.sessionId;
      const identity = requestId
        ? permissionIdentity(channelId, requestSessionId, ctx.turnId, requestId)
        : `${ch}:${requestSessionId ?? "unknown-session"}:${event.turnId ?? event.seq}:${event.seq}`;
      const itemId = `permission:${identity}`;
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
      if (requestId) {
        d.pendingPermissions = new Map(d.pendingPermissions);
        d.pendingPermissions.set(
          permissionIdentity(
            channelId,
            requestSessionId,
            ctx.turnId,
            requestId,
          ),
          {
            itemId,
            optionNames: request.optionNames,
          },
        );
      }
    } else if (event.kind === "acp_write" && !method) {
      // Permission response: {"id": <same as request>, "result": {"outcome": {...}}}
      const responseId = jsonRpcId(payload.id);
      const result = asRecord(asRecord(payload.result).outcome);
      const outcomeKind = asString(result.outcome);
      const responseKey = responseId
        ? permissionIdentity(channelId, ctx.sessionId, ctx.turnId, responseId)
        : null;
      const pending = responseKey
        ? d.pendingPermissions.get(responseKey)
        : null;
      if (pending && outcomeKind && responseKey) {
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
            pendingResolution: undefined,
          });
          // Remove from pending map — the outcome is now recorded.
          d.pendingPermissions = new Map(d.pendingPermissions);
          d.pendingPermissions.delete(responseKey);
        }
      }
    } else if (event.kind === "acp_write" && method === "session/prompt") {
      const promptBlocks = extractPromptBlocks(payload);
      if (promptBlocks.length > 0) {
        const parsedPrompt = parsePromptBlocks(promptBlocks);
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
            parsedPrompt.userEventId ??
              getSingleTriggeringEventId(d, ch, event.turnId ?? event.seq),
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
      // the harness as <base>/<agent-instructions>/<core-memory>/<channel-canvas>.
      // ACP adapters may use a replacement string or a replace/append object
      // under _meta.systemPrompt. All paths produce the same standalone card
      // (turnId: null, acpSource "session/new");
      // the bare field takes precedence when both are present.
      const params = asRecord(payload.params);
      const metaSystemPrompt = asRecord(params._meta).systemPrompt;
      const metaPrompt =
        asString(metaSystemPrompt) ??
        asString(asRecord(metaSystemPrompt).replace) ??
        asString(asRecord(metaSystemPrompt).append);
      const systemPrompt = asString(params.systemPrompt) ?? metaPrompt;
      if (systemPrompt) {
        const sections = parseSystemPromptSections(systemPrompt);
        if (sections.length > 0) {
          upsertMetadata(
            d,
            `system-prompt:${ch}:${event.seq}:${event.timestamp}`,
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
      const promptBlocks = extractPromptBlocks(payload);
      if (promptBlocks.length > 0) {
        const parsedPrompt = parsePromptBlocks(promptBlocks);
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
            "session/steer:user",
            parsedPrompt.userEventId,
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
          const channelMessageId = maybeNostrEventId(messageId);
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
            channelMessageId,
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
          extractPlanText(update),
          event.timestamp,
          ctx,
          updateType,
          `plan-update:${ch}:${turnKey}:${event.seq}`,
        );
      } else if (updateType === "current_mode_update") {
        const mode = asString(update.currentModeId) ?? "";
        if (mode) {
          upsertLifecycleItem(
            d,
            `mode:${ch}:${turnKey}`,
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
            `usage:${ch}:${turnKey}`,
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
          `commands:${ch}:${turnKey}`,
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
            `config:${ch}:${turnKey}`,
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
            `status:${ch}:${event.turnId ?? event.seq}:${status.statusType}`,
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
          `status:${ch}:${event.turnId ?? event.seq}:${status.statusType}`,
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

  if (!d.changed && d.latestSessionId === state.latestSessionId) {
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
    latestSessionId: d.latestSessionId,
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
