import type { ObserverEvent } from "../ui/agentSessionTypes";
import {
  CAUSAL_EXPERIMENT_SCHEMA,
  CausalLedger,
  type CausalExperiment,
} from "./causalLedger";

type JournalStorage = Pick<Storage, "getItem" | "setItem">;

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
  readonly #storage: JournalStorage;
  readonly #owner: string;
  readonly #sessions = new Map<string, ObserverEvent[]>();
  #ledger = new CausalLedger();
  #ready: Promise<void>;
  #writes: Promise<void> = Promise.resolve();

  constructor(owner: string, storage: JournalStorage) {
    this.#owner = owner.toLowerCase();
    this.#storage = storage;
    this.#ready = this.#restore();
  }

  get storageKey(): string {
    return `buzz-causal-ledger.v1:${this.#owner}`;
  }

  async #restore() {
    const journal = this.#storage.getItem(this.storageKey);
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

      const experimentId = `live:${agentPubkey.toLowerCase()}:${event.sessionId}`;
      if (
        this.#ledger
          .entries()
          .some((entry) => entry.experiment.experimentId === experimentId)
      ) {
        this.#sessions.delete(key);
        return;
      }
      await this.#ledger.append(
        this.#candidate(experimentId, event.sessionId, event.turnId, session),
      );
      this.#storage.setItem(this.storageKey, this.#ledger.toJournal());
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
        description:
          text(taskPayload.description) ??
          "Task unavailable from observer evidence",
        sourceMessageId: text(taskPayload.sourceMessageId),
      },
      execution: { sessionId, turnId: turnId ?? "unknown", replayOf: null },
      failureFingerprint: "unclassified-live-candidate/v1",
      context: {
        codeVersion: "unknown",
        policyVersion: "unknown",
        modelVersion: "unknown",
        toolVersion: "buzz-acp-observer/v1",
        environmentVersion: "unknown",
      },
      hypothesis: { cause: "unclassified", evidenceIds: [] },
      intervention: {
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
