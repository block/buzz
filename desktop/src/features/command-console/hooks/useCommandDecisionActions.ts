import * as React from "react";

import { getActiveTurnsForAgent } from "@/features/agents/activeAgentTurnsStore";
import type { BriefDecision } from "@/features/command-console/domain/briefDecisions";
import { dispatchCommandDecision } from "@/features/command-console/domain/commandDecisionActions";
import { parseCommandDirectionStatus } from "@/features/command-console/domain/decisionDispatch";
import {
  DECISION_EXECUTION_STORAGE_KEY,
  type DecisionDirectionSource,
  type DecisionExecution,
  markSilentExecutionStalled,
  parseDecisionExecutions,
  serializeDecisionExecutions,
  updateDecisionExecution,
} from "@/features/command-console/domain/decisionExecutionStore";
import { relayClient } from "@/shared/api/relayClient";
import { sendChannelMessage } from "@/shared/api/tauri";

type ChiefConversation = Readonly<{ pubkey: string; channelId: string }>;
type LiveEvent = Readonly<{ pubkey: string; content: string }>;
type StorageLike = Pick<Storage, "getItem" | "setItem">;

export type CommandDecisionActionDependencies = Readonly<{
  openChief: () => Promise<ChiefConversation>;
  navigate: (channelId: string) => Promise<unknown>;
  send?: (message: {
    channelId: string;
    content: string;
    mentionPubkeys: readonly string[];
  }) => Promise<unknown>;
  subscribe?: (
    channelId: string,
    listener: (event: LiveEvent) => void,
  ) => Promise<() => void | Promise<void>>;
  hasActiveTurn?: (agentPubkey: string, channelId: string) => boolean;
  now?: () => number;
  storage?: StorageLike | null;
  setInterval?: (callback: () => void, delay: number) => number;
  clearInterval?: (handle: number) => void;
}>;

function browserStorage(): StorageLike | null {
  return typeof window === "undefined" ? null : window.localStorage;
}

function upsertExecution(
  executions: readonly DecisionExecution[],
  next: DecisionExecution,
) {
  const index = executions.findIndex((execution) => execution.key === next.key);
  if (index < 0) return [...executions, next];
  return executions.map((execution, candidateIndex) =>
    candidateIndex === index ? next : execution,
  );
}

export function useCommandDecisionActions(
  dependencies: CommandDecisionActionDependencies,
) {
  const storage =
    dependencies.storage === undefined
      ? browserStorage()
      : dependencies.storage;
  const now = dependencies.now ?? Date.now;
  const send =
    dependencies.send ??
    ((message) =>
      sendChannelMessage(
        message.channelId,
        message.content,
        undefined,
        undefined,
        [...message.mentionPubkeys],
      ));
  const subscribe =
    dependencies.subscribe ??
    ((channelId, listener) =>
      relayClient.subscribeToChannel(channelId, listener));
  const hasActiveTurn =
    dependencies.hasActiveTurn ??
    ((agentPubkey, channelId) =>
      getActiveTurnsForAgent(agentPubkey).some(
        (turn) => turn.channelId === channelId,
      ));
  const installInterval = dependencies.setInterval ?? globalThis.setInterval;
  const removeInterval = dependencies.clearInterval ?? globalThis.clearInterval;
  const [executions, setExecutions] = React.useState<
    readonly DecisionExecution[]
  >(() =>
    parseDecisionExecutions(
      storage?.getItem(DECISION_EXECUTION_STORAGE_KEY) ?? null,
    ),
  );
  const [pendingKeys, setPendingKeys] = React.useState<ReadonlySet<string>>(
    () => new Set(),
  );

  const onUpdate = React.useCallback((execution: DecisionExecution) => {
    setExecutions((current) => upsertExecution(current, execution));
  }, []);

  React.useEffect(() => {
    storage?.setItem(
      DECISION_EXECUTION_STORAGE_KEY,
      serializeDecisionExecutions(executions),
    );
  }, [executions, storage]);

  React.useEffect(() => {
    let disposed = false;
    const cleanups: Array<() => void | Promise<void>> = [];
    const live = executions.filter(
      (execution) =>
        execution.channelId &&
        execution.agentPubkey &&
        !["completed", "blocked", "failed"].includes(execution.status),
    );

    for (const execution of live) {
      void subscribe(execution.channelId ?? "", (event) => {
        if (disposed || event.pubkey !== execution.agentPubkey) return;
        const parsed = parseCommandDirectionStatus(
          event.content,
          execution.key,
        );
        if (!parsed) return;
        onUpdate(
          updateDecisionExecution(execution, {
            ...parsed,
            now: now(),
          }),
        );
      })
        .then((cleanup) => {
          if (disposed) void cleanup();
          else cleanups.push(cleanup);
        })
        .catch(() => {});
    }

    return () => {
      disposed = true;
      for (const cleanup of cleanups) void cleanup();
    };
  }, [executions, now, onUpdate, subscribe]);

  React.useEffect(() => {
    const handle = installInterval(() => {
      setExecutions((current) => {
        let changed = false;
        const next = current.map((execution) => {
          if (!execution.agentPubkey || !execution.channelId) {
            const stalled = markSilentExecutionStalled(execution, now());
            changed ||= stalled !== execution;
            return stalled;
          }
          if (
            !["completed", "blocked", "failed"].includes(execution.status) &&
            hasActiveTurn(execution.agentPubkey, execution.channelId)
          ) {
            const active = updateDecisionExecution(execution, {
              status: "in_progress",
              statusText: "Chief of Staff is working.",
              now: now(),
            });
            changed = true;
            return active;
          }
          const stalled = markSilentExecutionStalled(execution, now());
          changed ||= stalled !== execution;
          return stalled;
        });
        return changed ? next : current;
      });
    }, 15_000);
    return () => removeInterval(handle);
  }, [hasActiveTurn, installInterval, now, removeInterval]);

  const issue = React.useCallback(
    async (
      decision: BriefDecision,
      direction: string,
      directionSource: DecisionDirectionSource,
    ) => {
      setPendingKeys((current) => new Set(current).add(decision.key));
      try {
        await dispatchCommandDecision(
          { decision, direction, directionSource, now },
          {
            openChief: dependencies.openChief,
            send,
            onUpdate,
          },
        );
      } finally {
        setPendingKeys((current) => {
          const next = new Set(current);
          next.delete(decision.key);
          return next;
        });
      }
    },
    [dependencies.openChief, now, onUpdate, send],
  );

  const retry = React.useCallback(
    (decision: BriefDecision, execution: DecisionExecution) =>
      issue(decision, execution.direction, execution.directionSource),
    [issue],
  );

  const openConversation = React.useCallback(
    async (execution: DecisionExecution) => {
      if (execution.channelId) await dependencies.navigate(execution.channelId);
    },
    [dependencies.navigate],
  );

  return {
    executions,
    pendingKeys,
    issue,
    retry,
    openConversation,
  };
}
