import { useCallback, useEffect, useState } from "react";

import {
  connectWorldMonitorOauth,
  disconnectWorldMonitor,
  getWorldMonitorConnection,
  testWorldMonitorConnection,
  type WorldMonitorConnection,
} from "@/shared/api/tauriCommandBrief";

export function useWorldMonitorConnection() {
  const [connection, setConnection] = useState<WorldMonitorConnection | null>(
    null,
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const perform = useCallback(
    async (operation: () => Promise<WorldMonitorConnection>) => {
      setBusy(true);
      setError(null);
      try {
        const next = await operation();
        setConnection(next);
        return true;
      } catch (cause) {
        setError(
          cause instanceof Error
            ? cause.message
            : "World Monitor is unavailable.",
        );
        return false;
      } finally {
        setBusy(false);
      }
    },
    [],
  );

  useEffect(() => {
    void perform(getWorldMonitorConnection);
  }, [perform]);

  return {
    connection,
    busy,
    error,
    connect: () => perform(connectWorldMonitorOauth),
    disconnect: () => perform(disconnectWorldMonitor),
    test: () => perform(testWorldMonitorConnection),
  };
}
