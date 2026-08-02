/**
 * Types for the remote-agent surface: enumerating the user's own SSH hosts and
 * probing them for agent harnesses.
 *
 * A separate module rather than more lines in `types.ts`, which is already over
 * the desktop 1000-line limit and carries a documented "queued to be split"
 * override. Import these from here directly — `types.ts` deliberately does not
 * re-export them, because a re-export block would put it back over the limit
 * and defeat the point of the split.
 */

/** One `Host` stanza from the user's `~/.ssh/config`. */
export type SshHost = {
  /** The `Host` alias as written — this is what gets passed to `ssh`. */
  host: string;
  hostname?: string | null;
  user?: string | null;
  port?: string | null;
  identityFile?: string | null;
};

/**
 * Why a host probe failed, when the cause is actionable.
 *
 * `password_required` means the host offered only interactive auth. Buzz never
 * collects or stores an SSH password, so this is a status to render with a
 * remedy, not a prompt to raise.
 *
 * `host_key_problem` covers both an untrusted first-seen key and a changed one.
 * Buzz probes with strict host-key checking and never writes to `known_hosts`,
 * so granting trust is always something the user does outside the app.
 *
 * `truncated` means the probe started but its output stopped early, so the facts
 * are an unknown fraction of the real ones and are withheld rather than shown as
 * a complete answer.
 */
export type HostProbeErrorKind =
  | "password_required"
  | "host_key_problem"
  | "unreachable"
  | "timed_out"
  | "truncated";

/**
 * One agent harness found on a probed host.
 *
 * Deliberately narrower than `AcpRuntime`: that type carries install and auth
 * affordances that only apply to the local machine. Buzz does not install
 * software on, or authenticate CLIs on, another host.
 */
export type RemoteHarness = {
  id: string;
  label: string;
  source: "builtin" | "preset" | "custom";
  acpCommand?: string | null;
  acpCommandPath?: string | null;
  version?: string | null;
  underlyingCliPath?: string | null;
  /**
   * True when the harness is usable on this host: its ACP command resolved and,
   * if it is an adapter, the vendor CLI it wraps resolved too. An adapter
   * without its CLI starts and then fails at first use.
   */
  ready: boolean;
  installHint: string;
  installInstructionsUrl: string;
};

/**
 * Result of probing one host for agent harnesses.
 *
 * A host-side problem comes back with `ok: false` and a classified
 * `errorKind` rather than as a thrown error — the UI shows one row per host and
 * needs a renderable status.
 */
export type HostProbeResult = {
  /** The ssh alias probed, or `__localhost__` for this machine. */
  host: string;
  ok: boolean;
  durationMs: number;
  error?: string | null;
  errorKind?: HostProbeErrorKind | null;
  user?: string | null;
  hostname?: string | null;
  os?: string | null;
  /** Path of the `buzz` CLI on the host; a connected agent needs it. */
  buzzCliPath?: string | null;
  buzzCliVersion?: string | null;
  harnesses: RemoteHarness[];
};

/** Host id the backend uses for the local machine. */
export const LOCALHOST_HOST_ID = "__localhost__";

/** One durable, named agent reported by a harness. */
export type RemoteAgentCandidate = {
  /** Harness that reported this agent, matching `RemoteHarness.id`. */
  harnessId: string;
  /** Harness-owned routing key for this exact agent. */
  agentId: string;
  /** Best available label; falls back to `agentId`. */
  displayName: string;
  /** Whether the harness identifies this candidate as its primary agent. */
  isPrimary: boolean;
  model?: string;
  workspace?: string;
  /** Existing harness routing bindings, when reported. */
  bindingCount?: number;
};

/** Outcome of listing one harness's durable agents. */
export type HarnessRosterResult = {
  host: string;
  harnessId: string;
  ok: boolean;
  durationMs: number;
  /** False when Buzz has no roster recipe for this harness. */
  supported: boolean;
  error?: string;
  errorKind?: HostProbeErrorKind;
  candidates: RemoteAgentCandidate[];
};

/**
 * A self-hosted agent Buzz talks to but does not own: it runs on a machine the
 * user owns, supervises itself, and holds its own signing key.
 *
 * Deliberately **not** a `ManagedAgent`. That type carries `status`, `pid`,
 * `logPath`, `needsRestart`, and `startOnAppLaunch` — each one a claim about a
 * process Buzz supervises. A connected agent has none of those, and the narrow
 * shape is what makes "no start/stop button" a property of the type rather
 * than a rule a component has to remember. Connected agents are not part of
 * `listManagedAgents()` at all: they are a separate record type in a separate
 * store, so they cannot reach a surface that renders lifecycle controls.
 */
export type ConnectedAgent = {
  /** The agent's own pubkey, lowercase hex. Buzz holds only the public half. */
  pubkey: string;
  /** Buzz-local label. The agent's own kind:10100 profile is what the relay sees. */
  name: string;
  /** `~/.ssh/config` alias of the machine the agent and its key live on. */
  host: string;
  /**
   * Harness id observed on the host at connect time (e.g. `"claude"`). A
   * record of what was there — nothing in Buzz executes it.
   */
  harness: string | null;
  /** Community where this connection was created, or `null` for legacy records. */
  community: string | null;
  createdAt: string;
  updatedAt: string;
};
