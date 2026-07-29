import type { ManagedAgent, RemoteHarness } from "@/shared/api/types";
import { normalizePinnedCommand } from "./pinnedHarness";

/**
 * "Is this exclusive catalog entry already taken?"
 *
 * A remote harness entry marked `exclusive` names a persistent identity on the
 * host — its own memory, sessions and credentials — rather than an ephemeral
 * runner. Two agents pinned to the same one are two puppeteers driving one
 * body, so the create flow refuses the second. Everything here is provider- and
 * harness-agnostic: the entry says it is exclusive, and this decides whether
 * the machine + identity pair is already spoken for.
 *
 * Nothing here knows what Hermes, a profile, or SSH is. Do not teach it.
 */

/** The host an exclusive entry would be deployed to. */
export type RemoteDeployTarget = {
  /** Backend provider id — the `runOn` of the create draft. */
  providerId: string;
  /** The provider config the create would deploy with, already coerced. */
  config: Record<string, unknown>;
};

/**
 * Canonical form of a provider config, for equality only.
 *
 * Two records name the same machine when their provider AND their whole config
 * agree. Comparing the config wholesale rather than reading one blessed key
 * (`ssh_host`) is what keeps this generic — the desktop has no vocabulary for
 * "the host field" and would have to grow one per provider — and it is also
 * more correct for the provider that exists: the same address reached as a
 * different `ssh_user` is a different HOME, and therefore a different identity
 * store, so a host-only comparison would wrongly refuse it.
 *
 * Normalization is deliberately minimal: keys are sorted, string values are
 * trimmed, and empty strings are dropped so a blank optional field equals an
 * absent one (the create dialog seeds schema defaults into every draft, so both
 * spellings really do occur). Both sides of the comparison pass through
 * `coerceConfigValues` before they get here, so a numeric port is a number on
 * both.
 *
 * KNOWN LIMITATION, accepted for v1: this is exact matching, not host
 * resolution. `10.0.0.4`, `vps`, and `vps.tail1234.ts.net` are the same machine
 * to everyone except this function, and an explicit `ssh_port: 22` does not
 * equal an omitted one. Both mis-read as a DIFFERENT host, so the guard simply
 * does not fire and the user gets today's behavior — an unguarded second agent.
 * The failure mode is under-matching, never falsely blocking a legitimate
 * create. Resolving aliases needs a host-identity op in the provider protocol
 * (the provider is the only party that can answer "who did I just connect
 * to?"); that is the real fix, not a normalization table here.
 */
function canonicalConfig(config: Record<string, unknown>): string {
  const entries = Object.entries(config)
    .map(([key, value]) => {
      const normalized = typeof value === "string" ? value.trim() : value;
      return [key, normalized] as const;
    })
    .filter(
      ([, value]) => value !== "" && value !== null && value !== undefined,
    )
    .sort(([left], [right]) => (left < right ? -1 : left > right ? 1 : 0));
  return JSON.stringify(entries);
}

/**
 * The pinned identity of a harness: the command and args that will actually
 * run on the host.
 *
 * Canonicalized by `normalizePinnedCommand`, the same owner the pin's displayed
 * `command` string goes through — a rule that decides "same agent?" and a rule
 * that decides what the user reads must be one rule, or a pin differing only by
 * whitespace is taken here and a different string on screen.
 *
 * Beyond that normalization the comparison is literal, which is right for a
 * provider record: its create-time args are pinned verbatim precisely because
 * normalizing a REMOTE command against LOCAL runtime identity is a category
 * error (see `create_time_agent_args`). The summary layer still runs them
 * through `normalize_agent_args`, so an exclusive entry whose command happens
 * to share a basename with a known local runtime could read back with
 * substituted args and under-match here. No such entry exists today (`hermes`
 * is not a known local runtime), and the failure mode is the documented one:
 * the guard does not fire.
 */
function pinnedIdentity(command: string, args: readonly string[]): string {
  const pin = normalizePinnedCommand(command, args);
  return JSON.stringify([pin.command, pin.args]);
}

/**
 * Does an existing agent already occupy this exclusive entry?
 *
 * `false` for a non-exclusive entry, always and without inspecting anything —
 * an ephemeral runner is meant to be deployed N times, and that is the whole
 * point of the distinction.
 *
 * A record matches when all three hold:
 *   1. it is provider-backed by the SAME provider (a local agent runs on this
 *      computer and cannot occupy anything on the host),
 *   2. it deploys with the same provider config (see `canonicalConfig`), and
 *   3. its effective harness pin is byte-identical to the entry's command+args.
 *
 * `agentCommand` is the resolved/effective command, which is what a provider
 * record's deploy actually ships; `agentCommandOverride` can be null even for a
 * real remote pin when the pin happens to equal what the definition inherits,
 * so it is the wrong field to read here.
 */
export function isExclusiveRemoteHarnessAdded(
  harness: RemoteHarness,
  target: RemoteDeployTarget,
  agents: readonly ManagedAgent[],
): boolean {
  if (!harness.exclusive) return false;

  const targetHost = canonicalConfig(target.config);
  const targetIdentity = pinnedIdentity(harness.command, harness.args);

  return agents.some((agent) => {
    if (agent.backend.type !== "provider") return false;
    if (agent.backend.id !== target.providerId) return false;
    if (canonicalConfig(agent.backend.config) !== targetHost) return false;
    return (
      pinnedIdentity(agent.agentCommand, agent.agentArgs) === targetIdentity
    );
  });
}

/**
 * The ids of every catalog entry that is exclusive AND already added.
 *
 * The picker disables exactly these, and auto-pick skips exactly these, so both
 * read from one computation rather than each re-deriving the rule.
 */
export function addedExclusiveHarnessIds(
  harnesses: readonly RemoteHarness[],
  target: RemoteDeployTarget,
  agents: readonly ManagedAgent[],
): ReadonlySet<string> {
  return new Set(
    harnesses
      .filter((harness) =>
        isExclusiveRemoteHarnessAdded(harness, target, agents),
      )
      .map((harness) => harness.id),
  );
}
