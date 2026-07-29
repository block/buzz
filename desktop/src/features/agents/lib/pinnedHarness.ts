import { getHarnessLogoUrl } from "@/features/onboarding/ui/RuntimeIcon";
import type { ManagedAgent } from "@/shared/api/types";

/**
 * What a record's harness pin IS, read from the record and nothing else.
 *
 * For a provider-backed agent the pin (`agentCommand` + `agentArgs`) names a
 * binary on the HOST. This computer's runtime catalog has never seen it, so
 * every local lookup either misses outright (`hermes …` → no entry → generic
 * icon, "custom") or hits by pure name collision (`claude-agent-acp` happens
 * to be a local builtin's command, which is the only reason Claude's card ever
 * looked right). Neither is knowledge. The record is.
 *
 * The derivation is deliberately generic: a command basename, the same
 * base-id fallback `resolveHarnessLogo` already uses for variant ids, and a
 * `--profile <name>` flag — a widespread CLI convention, not one harness's
 * vocabulary. Nothing here knows what Hermes or SSH is. Do not teach it.
 */

/**
 * Human labels for every harness command this app has a name for.
 *
 * Mirrors the Rust `KNOWN_ACP_RUNTIMES` and `PRESET_HARNESSES` tables. The two
 * sides are different languages, so no compiler catches drift —
 * `pinnedHarness.test.mjs` reads the Rust source and asserts both directions,
 * exactly as `presetLogos.test.mjs` does for the logo map.
 *
 * A key with no Rust counterpart must be listed in that test's
 * `NOT_IN_RUST_CATALOG` with its reason: this table also names the free-form
 * command strings foreign surfaces carry (a relay agent's self-declared
 * `agentType`), which are harnesses Buzz itself cannot run.
 */
export const HARNESS_LABELS: Record<string, string> = {
  aider: "Aider",
  amp: "Amp",
  "buzz-agent": "Buzz Agent",
  claude: "Claude Code",
  codex: "Codex",
  cursor: "Cursor",
  goose: "Goose",
  grok: "Grok Build",
  hermes: "Hermes Agent",
  kimi: "Kimi Code",
  omp: "Oh My Pi",
  opencode: "OpenCode",
  openclaw: "OpenClaw",
};

/** Shown when a record carries no command at all. Matches the dialog copy. */
const UNCONFIGURED_LABEL = "Not configured";

/**
 * A pin's command and args in the one canonical spelling this app uses.
 *
 * The command is trimmed and blank args are dropped, because
 * `create_time_agent_args` drops them on the way into the record — a catalog
 * entry carrying one would otherwise never match the record minted from it.
 * Beyond that nothing is rewritten: the pin names a binary on the HOST, and
 * normalizing a remote command against local runtime identity is the category
 * error `create_time_agent_args` exists to avoid.
 *
 * One owner because two lanes read this: the pin's own `command` string (what a
 * human sees and copies) and `exclusiveRemoteHarness`'s identity comparison
 * (whether two records are the same agent). If they disagreed, a pin differing
 * only by whitespace would be "already added" and a different string on screen.
 */
export function normalizePinnedCommand(
  command: string,
  args: readonly string[],
): { command: string; args: string[] } {
  return {
    command: command.trim(),
    args: args.map((arg) => arg.trim()).filter((arg) => arg.length > 0),
  };
}

/** The identity of a harness pin, derived from the pin alone. */
export type PinnedHarness = {
  /**
   * The harness id the pin resolves to, or `null` when nothing is recognized —
   * an unknown host binary is a legitimate pin, and guessing an id for it would
   * be the same lie in a new place.
   */
  id: string | null;
  /** Display name for the pin. Never empty. */
  label: string;
  /** Bundled logo for the pin's harness, or `null` when it has none. */
  logoUrl: string | null;
  /** The pin exactly as it runs on the host: command followed by its args. */
  command: string;
};

/**
 * The harness name inside a command, stripped of the path and extension that
 * differ per host.
 *
 * Both separators are handled because the pin describes the HOST's filesystem,
 * which is not necessarily this computer's.
 */
function commandBasename(command: string): string {
  const trimmed = command.trim();
  const separator = Math.max(
    trimmed.lastIndexOf("/"),
    trimmed.lastIndexOf("\\"),
  );
  const basename = separator >= 0 ? trimmed.slice(separator + 1) : trimmed;
  return basename.replace(/\.(?:exe|cmd|bat)$/i, "").toLowerCase();
}

/**
 * The harness id a command basename belongs to.
 *
 * Exact first, then the text before the FIRST hyphen and only when that base is
 * itself a known id — the same rule `resolveHarnessLogo` applies to variant
 * ids, for the same reason. It resolves every real adapter spelling
 * (`hermes-acp` → `hermes`, `claude-agent-acp` → `claude`, `amp-acp` → `amp`,
 * `cursor-agent` → `cursor`) while leaving `buzz-agent` — a known id in its own
 * right — whole, and refusing to shorten an unknown command into an id it did
 * not earn.
 */
function resolveHarnessId(basename: string): string | null {
  if (basename in HARNESS_LABELS) return basename;
  const separator = basename.indexOf("-");
  if (separator <= 0) return null;
  const base = basename.slice(0, separator);
  return base in HARNESS_LABELS ? base : null;
}

const PROFILE_FLAG = "--profile";

/**
 * The profile a pin selects, when its args say so.
 *
 * A profile is a separate identity of the same harness — its own memory,
 * credentials and sessions — so `hermes --profile marshall acp` and a plain
 * `hermes` pin are two different agents and must not read as one name. Both
 * spellings of the flag are accepted; a value that looks like another flag is
 * refused, since that means the flag was passed without one.
 */
function profileName(args: readonly string[]): string | null {
  for (const [index, raw] of args.entries()) {
    const arg = raw.trim();
    if (arg.startsWith(`${PROFILE_FLAG}=`)) {
      const value = arg.slice(PROFILE_FLAG.length + 1).trim();
      if (value.length > 0) return value;
      continue;
    }
    if (arg !== PROFILE_FLAG) continue;
    const value = args[index + 1]?.trim();
    if (value && !value.startsWith("-")) return value;
  }
  return null;
}

/**
 * Identify a harness pin.
 *
 * The label names the harness a human recognizes, narrowed by profile when the
 * args name one. An unrecognized command falls back to the command itself
 * rather than to any local default: the pin is the truth, and a command we
 * cannot name is still better shown than replaced by a guess.
 */
export function resolvePinnedHarness(
  command: string,
  args: readonly string[],
): PinnedHarness {
  const pin = normalizePinnedCommand(command, args);
  const basename = commandBasename(pin.command);
  const id = resolveHarnessId(basename);
  const baseLabel = id ? HARNESS_LABELS[id] : pin.command || UNCONFIGURED_LABEL;
  const profile = profileName(args);

  return {
    id,
    label: profile ? `${baseLabel} (${profile})` : baseLabel,
    logoUrl: getHarnessLogoUrl(basename),
    command: [pin.command, ...pin.args].join(" ").trim(),
  };
}

/**
 * The harness pin of a provider-backed record, or `null` for a local one.
 *
 * The single owner of "may this surface read the record instead of the local
 * catalog?", so every surface asks the same question in the same place. `null`
 * means the local path stays exactly as it was: a local agent runs on this
 * computer, the catalog genuinely describes it, and its rendering must not
 * change.
 */
export function providerRecordHarness(
  agent: Pick<ManagedAgent, "backend" | "agentCommand" | "agentArgs">,
): PinnedHarness | null {
  if (agent.backend.type !== "provider") return null;
  return resolvePinnedHarness(agent.agentCommand, agent.agentArgs);
}
