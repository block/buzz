import {
  prepareWorkflowTrigger,
  triggerWorkflow,
} from "@/shared/api/tauriWorkflows";
import type { WorkflowTriggerScope } from "@/shared/api/tauriWorkflows";
import type { RelayEvent, TriggerWorkflowResponse } from "@/shared/api/types";

export type TriggerState = {
  status: "idle" | "pending" | "error" | "success";
  error?: string;
  failurePhase?: "prepare" | "submit";
  result?: TriggerWorkflowResponse;
};
const idle: TriggerState = { status: "idle" };
type Operation = {
  scope: Readonly<WorkflowTriggerScope>;
  state: TriggerState;
  event?: RelayEvent;
  inFlight?: Promise<TriggerWorkflowResponse>;
};

/** One session-lifetime owner across every view. Never evict ambiguous work.
 * No disk journal: retry continuity ends when the app is closed/reloaded.
 */
export class WorkflowTriggerOperations {
  private operations = new Map<string, Operation>();
  private listeners = new Set<() => void>();
  private readonly api;
  constructor(api = { prepareWorkflowTrigger, triggerWorkflow }) {
    this.api = api;
  }
  key(workflowId: string, scope: WorkflowTriggerScope) {
    return JSON.stringify([
      scope.expectedRelayUrl
        .trim()
        .replace(/^ws:/, "http:")
        .replace(/^wss:/, "https:")
        .replace(/\/+$/, ""),
      scope.expectedSignerPubkey.trim().toLowerCase(),
      workflowId,
    ]);
  }
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  state(key: string): TriggerState {
    return this.operations.get(key)?.state ?? idle;
  }
  private notify() {
    for (const listener of this.listeners) listener();
  }

  run(
    workflowId: string,
    scope: WorkflowTriggerScope,
    newRun = false,
  ): Promise<TriggerWorkflowResponse> {
    const key = this.key(workflowId, scope);
    let op = this.operations.get(key);
    if (op?.inFlight) return op.inFlight;
    // A settled success means the next explicit Trigger is a distinct run.
    // Submission errors retry the same event unless the user explicitly opts out.
    // Preparation errors have not published anything and may prepare again.
    if (!op || newRun || op.state.status === "success") {
      if (!op && this.operations.size >= 256) {
        for (const [oldKey, old] of this.operations) {
          if (old.state.status === "success") this.operations.delete(oldKey);
        }
        if (this.operations.size >= 256)
          return Promise.reject(
            new Error(
              "Too many unresolved workflow triggers. Retry existing operations before starting more.",
            ),
          );
      }
      op = { state: idle, scope: Object.freeze({ ...scope }) };
      this.operations.set(key, op);
    }
    const operation = op;
    const capturedScope = operation.scope;
    operation.state = { status: "pending" };
    // Schedule after inFlight is assigned: reentrant listeners and overlapping
    // card/editor clicks join the same operation, including during preparation.
    operation.inFlight = Promise.resolve().then(async () => {
      try {
        operation.event ??= await this.api.prepareWorkflowTrigger(
          workflowId,
          capturedScope,
        );
        const result = await this.api.triggerWorkflow(
          workflowId,
          operation.event,
          capturedScope,
        );
        operation.state = { status: "success", result };
        return result;
      } catch (error) {
        operation.state = {
          status: "error",
          error: error instanceof Error ? error.message : String(error),
          failurePhase: operation.event ? "submit" : "prepare",
        };
        throw error;
      } finally {
        operation.inFlight = undefined;
        this.notify();
      }
    });
    this.notify();
    return operation.inFlight;
  }
}

// Deliberately retained across A→B→A. Scope is part of every key and is
// asserted by both native commands; never show A's state in B.
export const workflowTriggerOperations = new WorkflowTriggerOperations();
