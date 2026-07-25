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

export type CommandBriefSchedulePatch =
  | Readonly<{
      enabled: boolean;
      localTime?: never;
      concurrency?: never;
    }>
  | Readonly<{
      enabled?: never;
      localTime: string;
      concurrency?: never;
    }>
  | Readonly<{
      enabled?: never;
      localTime?: never;
      concurrency: 1 | 2;
    }>;

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

const TERMINAL_STATES = new Set<BriefRunStatus["state"]>([
  "completed",
  "degraded",
  "cancelled",
  "failed",
]);

function mergeRunHistory(
  runId: string,
  ...groups: readonly (readonly BriefRunStatus[])[]
): readonly BriefRunStatus[] {
  const bySequence = new Map<number, BriefRunStatus>();
  for (const group of groups) {
    for (const entry of group) {
      if (entry.runId === runId && !bySequence.has(entry.sequence)) {
        bySequence.set(entry.sequence, entry);
      }
    }
  }
  return Object.freeze(
    [...bySequence.values()]
      .sort((left, right) => left.sequence - right.sequence)
      .slice(-32),
  );
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
  const mountedRef = React.useRef(false);
  const statusRef = React.useRef<BriefRunStatus | null>(null);
  const historyRef = React.useRef<readonly BriefRunStatus[]>([]);
  const mutationGenerationRef = React.useRef(0);
  const refreshGenerationRef = React.useRef(0);
  const startGenerationRef = React.useRef(0);
  const latestGenerationRef = React.useRef(0);
  const busyCountRef = React.useRef(0);
  const scheduleRef = React.useRef<BriefSchedule | null>(null);
  const desiredScheduleRef = React.useRef<CommandBriefScheduleUpdate | null>(
    null,
  );
  const scheduleRevisionRef = React.useRef(0);
  const scheduleWriteChainRef = React.useRef<Promise<void>>(Promise.resolve());

  const beginBusy = React.useCallback(() => {
    busyCountRef.current += 1;
    if (mountedRef.current) setBusy(true);
  }, []);

  const endBusy = React.useCallback(() => {
    busyCountRef.current = Math.max(0, busyCountRef.current - 1);
    if (mountedRef.current && busyCountRef.current === 0) setBusy(false);
  }, []);

  const commitRun = React.useCallback(
    (
      next: BriefRunStatus,
      incomingHistory: readonly BriefRunStatus[],
      options: {
        readonly allowSwitch: boolean;
        readonly allowBackfill: boolean;
      },
    ): boolean => {
      const current = statusRef.current;
      if (
        current &&
        current.runId !== next.runId &&
        !options.allowSwitch &&
        !TERMINAL_STATES.has(current.state)
      ) {
        return false;
      }
      if (
        current?.runId === next.runId &&
        next.sequence <= current.sequence &&
        !options.allowBackfill
      ) {
        return false;
      }
      const sameRun = current?.runId === next.runId;
      const merged = mergeRunHistory(
        next.runId,
        sameRun ? historyRef.current : [],
        incomingHistory,
        [next],
      );
      const newest =
        sameRun && current.sequence >= next.sequence ? current : next;
      if (
        sameRun &&
        newest === current &&
        merged.length === historyRef.current.length &&
        merged.every((entry, index) => entry === historyRef.current[index])
      ) {
        return false;
      }
      statusRef.current = newest;
      historyRef.current = merged;
      mutationGenerationRef.current += 1;
      if (mountedRef.current) {
        setStatus(newest);
        setHistory(merged);
      }
      return true;
    },
    [],
  );

  const loadLatestForTerminal = React.useCallback(
    async (terminalStatus: BriefRunStatus) => {
      const request = ++latestGenerationRef.current;
      try {
        const brief = await dependencies.getLatest();
        const current = statusRef.current;
        if (
          !mountedRef.current ||
          request !== latestGenerationRef.current ||
          current?.runId !== terminalStatus.runId ||
          current.sequence < terminalStatus.sequence ||
          brief?.brief.runId !== terminalStatus.runId
        ) {
          return;
        }
        setLatest(brief);
      } catch {
        if (mountedRef.current && request === latestGenerationRef.current) {
          setError(DISPLAY_ERROR);
        }
      }
    },
    [dependencies],
  );

  const refresh = React.useCallback(async () => {
    const request = ++refreshGenerationRef.current;
    const startingMutation = mutationGenerationRef.current;
    const startingScheduleRevision = scheduleRevisionRef.current;
    if (mountedRef.current) setLoading(true);
    try {
      const [nextStatus, nextLatest, nextSchedule] = await Promise.all([
        dependencies.getStatus(),
        dependencies.getLatest(),
        dependencies.getSchedule(),
      ]);
      if (!mountedRef.current || request !== refreshGenerationRef.current) {
        return;
      }
      const desired = desiredScheduleRef.current;
      const visibleSchedule =
        scheduleRevisionRef.current === startingScheduleRevision || !desired
          ? nextSchedule
          : Object.freeze({ ...nextSchedule, ...desired });
      scheduleRef.current = visibleSchedule;
      if (!desired) {
        desiredScheduleRef.current = {
          enabled: nextSchedule.enabled,
          localTime: nextSchedule.localTime,
          concurrency: nextSchedule.concurrency,
        };
      }
      setSchedule(visibleSchedule);
      const changedWhilePending =
        mutationGenerationRef.current !== startingMutation;
      if (nextStatus.current) {
        const current = statusRef.current;
        const sameRun = current?.runId === nextStatus.current.runId;
        if (!changedWhilePending || sameRun || current === null) {
          commitRun(nextStatus.current, nextStatus.history, {
            allowSwitch: !changedWhilePending,
            allowBackfill: true,
          });
        }
      } else if (!changedWhilePending) {
        statusRef.current = null;
        historyRef.current = Object.freeze([]);
        setStatus(null);
        setHistory(historyRef.current);
      }
      if (!changedWhilePending) {
        setLatest(nextLatest);
      } else if (
        nextLatest &&
        TERMINAL_STATES.has(statusRef.current?.state ?? "queued") &&
        nextLatest.brief.runId === statusRef.current?.runId
      ) {
        setLatest(nextLatest);
      }
      setError(null);
    } catch {
      if (mountedRef.current && request === refreshGenerationRef.current) {
        setError(DISPLAY_ERROR);
      }
    } finally {
      if (mountedRef.current && request === refreshGenerationRef.current) {
        setLoading(false);
      }
    }
  }, [commitRun, dependencies]);

  React.useEffect(() => {
    mountedRef.current = true;
    let disposed = false;
    let stop: (() => void) | null = null;
    void dependencies
      .subscribeStatus((next) => {
        if (disposed) return;
        const current = statusRef.current;
        const accepted = commitRun(next, [next], {
          allowSwitch:
            current === null ||
            TERMINAL_STATES.has(current.state) ||
            current.runId === next.runId,
          allowBackfill: false,
        });
        if (
          accepted &&
          (next.state === "completed" || next.state === "degraded")
        ) {
          void loadLatestForTerminal(next);
        }
      })
      .then((unlisten) => {
        if (disposed) unlisten();
        else {
          stop = unlisten;
          void refresh();
        }
      })
      .catch(() => {
        if (!disposed) {
          setError(DISPLAY_ERROR);
          setLoading(false);
        }
      });
    return () => {
      disposed = true;
      mountedRef.current = false;
      refreshGenerationRef.current += 1;
      startGenerationRef.current += 1;
      latestGenerationRef.current += 1;
      stop?.();
    };
  }, [commitRun, dependencies, loadLatestForTerminal, refresh]);

  const start = React.useCallback(async () => {
    const request = ++startGenerationRef.current;
    beginBusy();
    try {
      const next = await dependencies.start();
      if (mountedRef.current && request === startGenerationRef.current) {
        commitRun(next, [next], {
          allowSwitch: true,
          allowBackfill: true,
        });
        setError(null);
      }
      return next;
    } catch {
      if (mountedRef.current && request === startGenerationRef.current) {
        setError(DISPLAY_ERROR);
      }
      return null;
    } finally {
      endBusy();
    }
  }, [beginBusy, commitRun, dependencies, endBusy]);

  const cancel = React.useCallback(async () => {
    const current = statusRef.current;
    if (!current) return null;
    beginBusy();
    try {
      const next = await dependencies.cancel(current.runId);
      if (mountedRef.current && statusRef.current?.runId === current.runId) {
        commitRun(next, [next], {
          allowSwitch: false,
          allowBackfill: false,
        });
        setError(null);
      }
      return next;
    } catch {
      if (mountedRef.current) setError(DISPLAY_ERROR);
      return null;
    } finally {
      endBusy();
    }
  }, [beginBusy, commitRun, dependencies, endBusy]);

  const updateSchedule = React.useCallback(
    (patch: CommandBriefSchedulePatch) => {
      const current =
        desiredScheduleRef.current ??
        (scheduleRef.current
          ? {
              enabled: scheduleRef.current.enabled,
              localTime: scheduleRef.current.localTime,
              concurrency: scheduleRef.current.concurrency,
            }
          : null);
      if (!current) return Promise.resolve(null);
      const desired = Object.freeze({ ...current, ...patch });
      desiredScheduleRef.current = desired;
      scheduleRevisionRef.current += 1;
      if (scheduleRef.current && mountedRef.current) {
        const optimistic = Object.freeze({
          ...scheduleRef.current,
          ...desired,
        });
        scheduleRef.current = optimistic;
        setSchedule(optimistic);
      }
      beginBusy();
      const write = scheduleWriteChainRef.current.then(async () => {
        const submitted = desiredScheduleRef.current;
        if (!submitted) return null;
        try {
          const response = await dependencies.setSchedule(submitted);
          if (mountedRef.current) {
            const stillDesired = desiredScheduleRef.current ?? submitted;
            const visible = Object.freeze({
              ...response,
              ...stillDesired,
            });
            scheduleRef.current = visible;
            setSchedule(visible);
            setError(null);
          }
          return response;
        } catch {
          if (mountedRef.current) setError(DISPLAY_ERROR);
          return null;
        } finally {
          endBusy();
        }
      });
      scheduleWriteChainRef.current = write.then(
        () => undefined,
        () => undefined,
      );
      return write;
    },
    [beginBusy, dependencies, endBusy],
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
