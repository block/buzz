import { listen } from "@tauri-apps/api/event";
import * as React from "react";

import {
  parseBriefRunStatus,
  type BriefRunStatus,
  type BriefSchedule,
  type PublishedCommandBrief,
} from "@/features/command-console/domain/briefContracts";
import {
  cancelCommandBrief,
  getCommandBriefSchedule,
  getCommandBriefStatus,
  getLatestCommandBrief,
  setCommandBriefSchedule,
  startCommandBrief,
  type CommandBriefScheduleUpdate,
  type CommandBriefStatusView,
} from "@/shared/api/tauriCommandBrief";

export type DailyCommandBriefDependencies = {
  readonly getStatus: () => Promise<CommandBriefStatusView>;
  readonly getLatest: () => Promise<PublishedCommandBrief | null>;
  readonly getSchedule: () => Promise<BriefSchedule>;
  readonly start: () => Promise<BriefRunStatus>;
  readonly cancel: (runId: string) => Promise<BriefRunStatus>;
  readonly setSchedule: (
    update: CommandBriefScheduleUpdate,
  ) => Promise<BriefSchedule>;
  readonly subscribeStatus: (
    listener: (status: BriefRunStatus) => void,
  ) => Promise<() => void>;
};

const defaultDependencies: DailyCommandBriefDependencies = {
  getStatus: getCommandBriefStatus,
  getLatest: getLatestCommandBrief,
  getSchedule: getCommandBriefSchedule,
  start: startCommandBrief,
  cancel: cancelCommandBrief,
  setSchedule: setCommandBriefSchedule,
  subscribeStatus: async (listener) =>
    listen<unknown>("command-brief-status-changed", (event) => {
      const status = parseBriefRunStatus(event.payload);
      if (status) listener(status);
    }),
};

const DISPLAY_ERROR = "Daily Command Brief is unavailable.";

function appendStatus(
  current: readonly BriefRunStatus[],
  next: BriefRunStatus,
): readonly BriefRunStatus[] {
  const previous = current.at(-1);
  if (
    previous?.runId === next.runId &&
    previous.state === next.state &&
    previous.updatedAt === next.updatedAt
  ) {
    return current;
  }
  return Object.freeze([...current.slice(-31), next]);
}

export function useDailyCommandBrief(
  dependencies: DailyCommandBriefDependencies = defaultDependencies,
) {
  const [status, setStatus] = React.useState<BriefRunStatus | null>(null);
  const [history, setHistory] = React.useState<readonly BriefRunStatus[]>([]);
  const [latest, setLatest] = React.useState<PublishedCommandBrief | null>(
    null,
  );
  const [schedule, setSchedule] = React.useState<BriefSchedule | null>(null);
  const [loading, setLoading] = React.useState(true);
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  const refresh = React.useCallback(async () => {
    setLoading(true);
    try {
      const [nextStatus, nextLatest, nextSchedule] = await Promise.all([
        dependencies.getStatus(),
        dependencies.getLatest(),
        dependencies.getSchedule(),
      ]);
      setStatus(nextStatus.current);
      setHistory(nextStatus.history);
      setLatest(nextLatest);
      setSchedule(nextSchedule);
      setError(null);
    } catch {
      setError(DISPLAY_ERROR);
    } finally {
      setLoading(false);
    }
  }, [dependencies]);

  React.useEffect(() => {
    void refresh();
    let disposed = false;
    let stop: (() => void) | null = null;
    void dependencies
      .subscribeStatus((next) => {
        if (disposed) return;
        setStatus(next);
        setHistory((current) => appendStatus(current, next));
        if (next.state === "completed" || next.state === "degraded") {
          void dependencies
            .getLatest()
            .then((brief) => {
              if (!disposed) setLatest(brief);
            })
            .catch(() => {
              if (!disposed) setError(DISPLAY_ERROR);
            });
        }
      })
      .then((unlisten) => {
        if (disposed) unlisten();
        else stop = unlisten;
      })
      .catch(() => {
        if (!disposed) setError(DISPLAY_ERROR);
      });
    return () => {
      disposed = true;
      stop?.();
    };
  }, [dependencies, refresh]);

  const start = React.useCallback(async () => {
    setBusy(true);
    try {
      const next = await dependencies.start();
      setStatus(next);
      setHistory(Object.freeze([next]));
      setError(null);
      return next;
    } catch {
      setError(DISPLAY_ERROR);
      return null;
    } finally {
      setBusy(false);
    }
  }, [dependencies]);

  const cancel = React.useCallback(async () => {
    if (!status) return null;
    setBusy(true);
    try {
      const next = await dependencies.cancel(status.runId);
      setStatus(next);
      setHistory((current) => appendStatus(current, next));
      setError(null);
      return next;
    } catch {
      setError(DISPLAY_ERROR);
      return null;
    } finally {
      setBusy(false);
    }
  }, [dependencies, status]);

  const updateSchedule = React.useCallback(
    async (update: CommandBriefScheduleUpdate) => {
      setBusy(true);
      try {
        const next = await dependencies.setSchedule(update);
        setSchedule(next);
        setError(null);
        return next;
      } catch {
        setError(DISPLAY_ERROR);
        return null;
      } finally {
        setBusy(false);
      }
    },
    [dependencies],
  );

  return {
    status,
    history,
    latest,
    schedule,
    loading,
    busy,
    error,
    refresh,
    start,
    cancel,
    updateSchedule,
  } as const;
}
