/**
 * Types for backend providers: the `buzz-backend-*` binaries that run an agent
 * somewhere other than this computer.
 *
 * Kept out of `types.ts` because everything here describes a REMOTE machine,
 * and conflating it with the local-runtime vocabulary is precisely the mistake
 * that makes a remote agent silently deploy the wrong harness.
 */

export type BackendProviderCandidate = {
  id: string;
  binaryPath: string;
};

export type BackendProviderProbeResult = {
  ok: boolean;
  name?: string;
  version?: string;
  description?: string;
  config_schema?: Record<string, unknown>;
};

/**
 * One harness on the machine a provider deploys to, from the provider's
 * `discover_harnesses` op. `command`/`args`/`env` describe the REMOTE host, so
 * they are pinned onto the agent record verbatim at create time — nothing
 * re-resolves them, because a provider-backed agent never spawns locally.
 */
export type RemoteHarness = {
  id: string;
  label: string;
  command: string;
  args: string[];
  env: Record<string, string>;
  available: boolean;
  binaryPath: string | null;
  version: string | null;
  /**
   * The entry names a persistent IDENTITY on the host rather than an ephemeral
   * runner, so at most one agent may be pinned to it.
   *
   * Absent (the default, and what every entry emitted before this field looked
   * like) means the opposite: deploying the same harness N times against one
   * host is the point of a runner, and nothing about it is scarce. A provider
   * sets this when a second agent on the same entry would share one identity's
   * memory, sessions and credentials — two puppeteers on one body. The desktop
   * knows nothing about WHICH harnesses those are; it only refuses to add a
   * marked one twice (`isExclusiveRemoteHarnessAdded`).
   */
  exclusive?: boolean;
};

export type RemoteHarnessCatalog = {
  /** `null` when buzz-acp is not installed on the host — actionable, not fatal. */
  buzzAcp: { path: string; version: string } | null;
  harnesses: RemoteHarness[];
};
