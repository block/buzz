import type { ObserverEvent } from "../ui/agentSessionTypes";
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
      const key = `${agentPubkey.toLowerCase()}:${event.sessionId}`;
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

      const experimentId = `live:${agentPubkey.toLowerCase()}:${event.sessionId}`;
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
    const taskEvent = events.find((event) => event.kind === "task_captured");
    const taskPayload = taskEvent ? payload(taskEvent) : {};
    const taskDescription =
      text(taskPayload.description) ??
      "Task unavailable from observer evidence";
    const sourceMessageId = text(taskPayload.sourceMessageId);
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
        os_sandbox: "missing",
      },
      relations: { supports: [], contradicts: [], invalidates: [] },
    };
  }
}
