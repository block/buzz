export type ManagedAgentRuntimeLifecycle =
  | "starting"
  | "listening"
  | "waking"
  | "ready"
  | "recovering"
  | "legacy_runtime_active"
  | "manual_legacy_stop_required"
  | "failed"
  | "stopped";

export type ManagedAgentAssignmentState =
  | "reading"
  | "working"
  | "waiting"
  | "needs_approval"
  | "blocked"
  | "recovering"
  | "completed"
  | "failed"
  | "cancelled";

export type ManagedAgentActiveAssignment = {
  assignmentId: string;
  channelId: string;
  sourceEventId: string | null;
  state: ManagedAgentAssignmentState;
  summary: string;
  activeJobId: string | null;
  lastProgressAt: string;
  hasBlocker: boolean;
};

export type ManagedAgentActiveJob = {
  jobId: string;
  requestEventId: string | null;
  sourceEventId: string | null;
  channelId: string;
  state:
    | "requested"
    | "accepted"
    | "running"
    | "cancelling"
    | "succeeded"
    | "failed"
    | "cancelled"
    | "lost";
  attempt: number;
  progressSeq: number;
  summary: string;
  startedAt: string | null;
  finishedAt: string | null;
  exitCode: number | null;
  errorCode: string | null;
  publicationState: "not_started" | "pending" | "published" | "failed";
  runnerPid: number | null;
  runnerStartMarker: string | null;
};

export type ManagedAgentRuntimeStatus = {
  pubkey: string;
  /** Exact submitted descriptor, present only on startup reconcile results. */
  requestedRelayUrl?: string;
  /** Canonical, backend-owned pair identity component. Do not normalize in TS. */
  relayUrl: string;
  localSetup: boolean;
  lifecycle: ManagedAgentRuntimeLifecycle;
  pid: number | null;
  error: string | null;
  logPath: string | null;
  activeAssignment?: ManagedAgentActiveAssignment | null;
  activeJob?: ManagedAgentActiveJob | null;
};
