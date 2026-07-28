import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useEffect } from "react";

import {
  requestOpenSnapshotImport,
  type PendingSnapshotImport,
} from "@/features/agents/openSnapshotImportFromUrlEvent";

type PendingAgentSnapshotImport = PendingSnapshotImport & {
  id: string;
  snapshotKind: "agent";
};

type AgentSnapshotHandoffDeps = {
  take: () => Promise<PendingAgentSnapshotImport | null>;
  acknowledge: (id: string) => Promise<boolean>;
  requestOpen: (payload: PendingSnapshotImport) => void;
  goAgents: () => unknown;
};

export async function drainPendingAgentSnapshotImport(
  deps: AgentSnapshotHandoffDeps,
): Promise<boolean> {
  const pending = await deps.take();
  if (!pending) return false;

  deps.requestOpen({
    fileBytes: pending.fileBytes,
    fileName: pending.fileName,
    snapshotKind: "agent",
  });
  await deps.goAgents();
  return deps.acknowledge(pending.id);
}

export async function listenForAgentSnapshotHandoffs(
  goAgents: () => unknown,
): Promise<UnlistenFn> {
  let drainRunning = false;
  let drainRequested = false;
  const drain = () => {
    drainRequested = true;
    if (drainRunning) return;
    drainRunning = true;
    void (async () => {
      try {
        while (drainRequested) {
          drainRequested = false;
          await drainPendingAgentSnapshotImport({
            take: () =>
              invoke<PendingAgentSnapshotImport | null>(
                "take_pending_agent_snapshot_import",
              ),
            acknowledge: (id) =>
              invoke<boolean>("acknowledge_pending_agent_snapshot_import", {
                id,
              }),
            requestOpen: requestOpenSnapshotImport,
            goAgents,
          });
        }
      } catch (error: unknown) {
        console.warn("Failed to drain pending agent snapshot handoff", error);
      } finally {
        drainRunning = false;
        if (drainRequested) drain();
      }
    })();
  };

  const unlisten = await listen("agent-snapshot-import-available", drain);
  drain();
  return unlisten;
}

export function useAgentSnapshotHandoffs(goAgents: () => unknown) {
  useEffect(() => {
    const unlisten = listenForAgentSnapshotHandoffs(goAgents);
    return () => {
      void unlisten.then((stop) => stop());
    };
  }, [goAgents]);
}
