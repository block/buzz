import type { ObserverEvent } from "../ui/agentSessionTypes";
import {
  extractPromptText,
  extractToolResult,
  parsePromptText,
} from "../ui/agentSessionTranscriptHelpers";
import { asRecord, asString } from "../ui/agentSessionUtils";
import {
  CAUSAL_EXPERIMENT_SCHEMA,
  CausalLedger,
  type CausalExperiment,
} from "./causalLedger";
import { proposalIdFromReplayTask } from "./causalReplayProposal";

export type CausalLedgerPersistence = {
  loadJournal(): Promise<string>;
  appendEntry(entry: import("./causalLedger").LedgerEntry): Promise<void>;
};

export function browserLedgerPersistence(
  owner: string,
  storage: Pick<Storage, "getItem" | "setItem">,
): CausalLedgerPersistence {
  const storageKey = `buzz-causal-ledger.v1:${owner.toLowerCase()}`;
  return {
    async loadJournal() {
      return storage.getItem(storageKey) ?? "";
    },
    async appendEntry(entry) {
      const existing = storage.getItem(storageKey);
      storage.setItem(
        storageKey,
        existing
          ? `${existing}\n${JSON.stringify(entry)}`
          : JSON.stringify(entry),
      );
    },
  };
}

function payload(event: ObserverEvent): Record<string, unknown> {
  return event.payload && typeof event.payload === "object"
    ? (event.payload as Record<string, unknown>)
    : {};
}

function text(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

function eventId(event: ObserverEvent): string {
  return `observer:${event.sessionId ?? "unknown"}:${event.seq}:${event.timestamp}`;
}

function taskFromEvents(events: readonly ObserverEvent[]): {
  description: string;
  sourceMessageId: string | null;
} {
  const captured = events.find((event) => event.kind === "task_captured");
  if (captured) {
    const capturedPayload = payload(captured);
    return {
      description:
        text(capturedPayload.description) ??
        "Task unavailable from observer evidence",
      sourceMessageId: text(capturedPayload.sourceMessageId),
    };
  }

  const promptEvent = events.find((event) => {
    const eventPayload = payload(event);
    return (
      event.kind === "acp_write" &&
      asString(eventPayload.method) === "session/prompt"
    );
  });
  if (!promptEvent) {
    return {
      description: "Task unavailable from observer evidence",
      sourceMessageId: null,
    };
  }
  const prompt = extractPromptText(payload(promptEvent));
  const parsed = parsePromptText(prompt);
  return {
    description:
      parsed.userText ||
      prompt.trim() ||
      "Task unavailable from observer evidence",
    sourceMessageId: parsed.userEventId,
  };
}

function hasOsSandboxEvidence(events: readonly ObserverEvent[]): boolean {
  const toolOutputById = new Map<string, string>();
  for (const event of events) {
    if (event.kind !== "acp_read") continue;
    const eventPayload = payload(event);
    if (asString(eventPayload.method) !== "session/update") continue;
    const update = asRecord(asRecord(eventPayload.params).update);
    if (asString(update.sessionUpdate) !== "tool_call_update") continue;
    const toolCallId = asString(update.toolCallId);
    const output = extractToolResult(update);
    if (toolCallId && output) toolOutputById.set(toolCallId, output);
    if (asString(update.status) !== "failed") continue;
    const detail = output || (toolCallId ? toolOutputById.get(toolCallId) : "");
    if (/permission denied|operation not permitted/i.test(detail ?? "")) {
      return true;
    }
  }
  return false;
}

export class LiveCausalLedger {
  readonly #persistence: CausalLedgerPersistence;
  readonly #owner: string;
  readonly #sessions = new Map<string, ObserverEvent[]>();
  #ledger = new CausalLedger();
  #ready: Promise<void>;
  #writes: Promise<void> = Promise.resolve();

  constructor(owner: string, persistence: CausalLedgerPersistence) {
    this.#owner = owner.toLowerCase();
    this.#persistence = persistence;
    this.#ready = this.#restore();
  }

  get storageKey(): string {
    return `buzz-causal-ledger.v1:${this.#owner}`;
  }

  async #restore() {
    const journal = await this.#persistence.loadJournal();
    if (journal) this.#ledger = await CausalLedger.fromJournal(journal);
  }

  ingest(agentPubkey: string, event: ObserverEvent): Promise<void> {
    this.#writes = this.#writes.then(async () => {
      await this.#ready;
      if (!event.sessionId) return;
      const runId = event.turnId ?? `terminal-${event.seq}`;
      const key = `${agentPubkey.toLowerCase()}:${event.sessionId}:${runId}`;
      const session = this.#sessions.get(key) ?? [];
      session.push(event);
      this.#sessions.set(key, session);
      if (event.kind !== "turn_completed" && event.kind !== "turn_error")
        return;

      // Owner actions can append approvals/evaluations while this live ingestor
      // remains mounted. Refresh before closing the turn so correlation and the
      // next hash are based on the same durable ledger the UI just changed.
      const latestJournal = await this.#persistence.loadJournal();
      this.#ledger = latestJournal
        ? await CausalLedger.fromJournal(latestJournal)
        : new CausalLedger();

      const experimentId = `live:${agentPubkey.toLowerCase()}:${event.sessionId}:${runId}`;
      if (
        this.#ledger
          .entries()
          .some((entry) => entry.experiment.experimentId === experimentId)
      ) {
        this.#sessions.delete(key);
        return;
      }
      const entry = await this.#ledger.append(
        this.#candidate(experimentId, event.sessionId, event.turnId, session),
      );
      await this.#persistence.appendEntry(entry);
      this.#sessions.delete(key);
    });
    return this.#writes;
  }

  async entries() {
    await this.#ready;
    await this.#writes;
    return this.#ledger.entries();
  }

  #candidate(
    experimentId: string,
    sessionId: string,
    turnId: string | null,
    events: ObserverEvent[],
  ): CausalExperiment {
    const task = taskFromEvents(events);
    const taskDescription = task.description;
    const sourceMessageId = task.sourceMessageId;
    const proposalId = proposalIdFromReplayTask(taskDescription);
    const markedProposal = proposalId
      ? this.#ledger
          .entries()
          .find((entry) => entry.experiment.experimentId === proposalId)
          ?.experiment
      : undefined;
    const hasDispatchReceipt = this.#ledger
      .entries()
      .some(
        (entry) =>
          entry.experiment.execution.replayOf ===
            markedProposal?.experimentId &&
          entry.experiment.execution.sessionId.startsWith("replay-dispatch:") &&
          entry.experiment.task.sourceMessageId === sourceMessageId,
      );
    const proposal = hasDispatchReceipt ? markedProposal : undefined;
    const layers = new Set(
      events.map((event) => {
        if (event.kind === "host_operation") return "host_workspace";
        if (event.kind === "permission_decision") return "acp_permission_gate";
        return "acp_observer";
      }),
    );
    return {
      schema: CAUSAL_EXPERIMENT_SCHEMA,
      experimentId,
      recordedAt: events.at(-1)?.timestamp ?? new Date().toISOString(),
      task: {
        description: proposal?.task.description ?? taskDescription,
        sourceMessageId,
      },
      execution: {
        sessionId,
        turnId: turnId ?? "unknown",
        replayOf: proposal?.experimentId ?? null,
      },
      failureFingerprint:
        proposal?.failureFingerprint ?? "unclassified-live-candidate/v1",
      context: proposal?.context ?? {
        codeVersion: "unknown",
        policyVersion: "unknown",
        modelVersion: "unknown",
        toolVersion: "buzz-acp-observer/v1",
        environmentVersion: "unknown",
      },
      hypothesis: proposal?.hypothesis ?? {
        cause: "unclassified",
        evidenceIds: [],
      },
      intervention: proposal?.intervention ?? {
        remedyId: "unclassified",
        changedVariable: "unclassified",
      },
      result: {
        outcome: "untested",
        evidenceIds: events.map(eventId),
      },
      coverage: {
        acp_observer: layers.has("acp_observer") ? "observed" : "missing",
        acp_permission_gate: layers.has("acp_permission_gate")
          ? "observed"
          : "missing",
        host_workspace: layers.has("host_workspace") ? "observed" : "missing",
        os_sandbox: hasOsSandboxEvidence(events) ? "observed" : "missing",
      },
      relations: { supports: [], contradicts: [], invalidates: [] },
    };
  }
}
