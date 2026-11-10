import type { ManagedAgentBackend, RespondToMode } from "@/shared/api/types";

/**
 * Where an agent's process runs, as far as the calling surface can tell.
 *
 * Deliberately coarser than `ManagedAgentBackend`: the warning copy only needs
 * to know "this machine" vs "somewhere else", so surfaces resolve their own
 * backend shape down to this before handing it over. `null` means the surface
 * genuinely cannot tell — see `agentAccessWarningText` for how that is
 * treated.
 */
export type AgentRunLocation = "local" | "remote";

/** Resolve a running agent's backend record. `null` when the backend is unknown. */
export function runLocationForBackend(
  backend: ManagedAgentBackend | null | undefined,
): AgentRunLocation | null {
  if (!backend) return null;
  return backend.type === "local" ? "local" : "remote";
}

/**
 * Resolve the create flow's `WhereToRunDraft.runOn`, which is `"local"` or a
 * discovered provider id. An empty string is treated as unknown rather than as
 * a provider, since `runOn` is typed `"local" | string`.
 */
export function runLocationForRunOn(
  runOn: string | null | undefined,
): AgentRunLocation | null {
  if (!runOn) return null;
  return runOn === "local" ? "local" : "remote";
}

/**
 * Copy for the shared-access warning in the respond-to field, or `null` for
 * modes that share nothing.
 *
 * Both `anyone` and `allowlist` hand the host's access to someone other than
 * the owner, so both warn; only the audience phrase differs.
 *
 * An unknown run location falls back to the same "your computer" wording as
 * `local` rather than hedging with "computer or server". A remote host is only
 * reachable when a `buzz-backend-*` provider binary is installed — without one
 * `WhereToRunSection`'s "Run on" selector never renders and every agent is
 * local — so hedging would name a concept most owners have never been shown.
 * When it *is* remote the owner picked that host from the selector
 * deliberately, so naming a server is meaningful there.
 */
export function agentAccessWarningText(
  mode: RespondToMode,
  runLocation?: AgentRunLocation | null,
): string | null {
  if (mode !== "anyone" && mode !== "allowlist") return null;
  const audience = mode === "anyone" ? "Anyone" : "Selected people";
  // The two locations differ in more than the noun: a local agent reaches the
  // owner's own files, while a remote host's files aren't theirs to describe —
  // only the accounts and tools provisioned there.
  const target =
    runLocation === "remote"
      ? "the server it runs on, including any accounts and tools available there"
      : "your computer, including files, accounts, and connected tools";
  return `${audience} can use this agent to access ${target}.`;
}
