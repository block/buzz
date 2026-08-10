export type ManagedAgentRuntimeLifecycle =
  | "starting"
  | "listening"
  | "waking"
  | "ready"
  | "failed"
  | "stopped";

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
};

export type ManagedAgentBackend =
  | { type: "local" }
  | { type: "provider"; id: string; config: Record<string, unknown> };

/** Owner-reviewed, instance-local filesystem boundary for local agents. */
export type FilesystemIsolationProfile = {
  mode: "ephemeral";
  /** Existing absolute directories exposed read-only in addition to runtime roots. */
  readOnlyRoots: string[];
};

export type FilesystemIsolationAttestation = {
  version: number;
  enforcement: string;
  identity_pubkey: string;
  run_id: string;
  run_root: string;
  allowed_read_roots: string[];
  allowed_write_roots: string[];
  denied_roots: string[];
};

export type PreparedFilesystemIsolation = {
  identityPubkey: string;
  runId: string;
  runRoot: string;
  attestation: FilesystemIsolationAttestation;
};

/** Inbound author gate mode. Mirrors buzz-acp's --respond-to CLI flag. */
export type RespondToMode = "owner-only" | "allowlist" | "anyone";
