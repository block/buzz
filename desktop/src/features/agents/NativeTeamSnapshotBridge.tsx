import * as React from "react";
import { isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useNavigate } from "@tanstack/react-router";

import { createNativeTeamSnapshotAcknowledgement } from "@/features/agents/nativeTeamSnapshotAcknowledgement";
import { requestNativeTeamSnapshotError } from "@/features/agents/nativeTeamSnapshotError";
import { requestOpenSnapshotImport } from "@/features/agents/openSnapshotImportFromUrlEvent";
import {
  acknowledgePendingNativeTeamSnapshot,
  readPendingNativeTeamSnapshot,
  takePendingNativeTeamSnapshot,
} from "@/shared/api/tauriTeams";

/** Drains native .buzzteam opens into the route-local Agents importer. */
export function NativeTeamSnapshotBridge() {
  const navigate = useNavigate();
  const drainRunningRef = React.useRef(false);
  const drainRequestedRef = React.useRef(false);
  const acknowledgementRef = React.useRef(
    createNativeTeamSnapshotAcknowledgement(
      acknowledgePendingNativeTeamSnapshot,
    ),
  );

  const goAgents = React.useCallback(
    () => navigate({ to: "/agents" }),
    [navigate],
  );

  const drain = React.useEffectEvent(async () => {
    if (!acknowledgementRef.current.requestDrain()) return;
    if (drainRunningRef.current) {
      drainRequestedRef.current = true;
      return;
    }
    drainRunningRef.current = true;
    try {
      while (true) {
        drainRequestedRef.current = false;
        const pending = await takePendingNativeTeamSnapshot();
        if (!pending) return;
        if (pending.error) {
          requestNativeTeamSnapshotError(
            { id: pending.id, message: pending.error },
            (id) => {
              void acknowledgementRef.current.acknowledge(id, () => {
                drainRequestedRef.current = true;
                void drain();
              });
            },
          );
          void goAgents();
          return;
        }
        try {
          const snapshot = await readPendingNativeTeamSnapshot(pending.id);
          await goAgents();
          requestOpenSnapshotImport({
            id: pending.id,
            fileBytes: snapshot.fileBytes,
            fileName: snapshot.fileName,
            snapshotKind: "team",
            onAccepted: () => {
              void acknowledgementRef.current.acknowledge(pending.id, () => {
                if (drainRequestedRef.current) void drain();
              });
            },
            onRejected: (id) => {
              void acknowledgementRef.current.acknowledge(id, () => {
                drainRequestedRef.current = true;
                void drain();
              });
            },
            onReleased: () => {
              drainRequestedRef.current = true;
              void drain();
            },
          });
        } catch (error) {
          requestNativeTeamSnapshotError(
            {
              id: pending.id,
              message:
                error instanceof Error
                  ? error.message
                  : "Failed to read opened team snapshot.",
            },
            (id) => {
              void acknowledgementRef.current.acknowledge(id, () => {
                drainRequestedRef.current = true;
                void drain();
              });
            },
          );
          void goAgents();
          return;
        }
        if (!drainRequestedRef.current) return;
      }
    } finally {
      drainRunningRef.current = false;
      if (drainRequestedRef.current) void drain();
    }
  });

  React.useEffect(() => {
    if (!isTauri()) return;
    const unlisten = listen("native-team-snapshot-opened", () => {
      drainRequestedRef.current = true;
      void drain();
    });
    drainRequestedRef.current = true;
    void drain();
    return () => {
      void unlisten.then((stop) => stop());
    };
  }, []);

  return null;
}
