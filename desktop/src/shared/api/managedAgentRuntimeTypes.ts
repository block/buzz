import type { ManagedAgentRuntimeLifecycle } from "./types";

export type ManagedAgentRuntimeStatus = {
  runId?: string | null;
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
};
