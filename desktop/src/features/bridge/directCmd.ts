// Desktop-side executor for the web's "desktop command proxy" over the direct (tailnet) connection.
// The web app runs the desktop's own Agents UI; its data commands (list_managed_agents, ...) are forwarded here by name.
// Only the commands in this table can run. Reads return the desktop's raw answer with secrets removed; writes forward only
// validated fields (never commands, args, env var values, prompts or key material). Pure: IO is injected via `invokeRaw`.
import { MgmtError } from "./mgmtError";

export type RawInvoke = (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;
export type CmdDeps = { invokeRaw: RawInvoke; ownerPubkey?: string };

const HEX64 = /^[0-9a-f]{64}$/;
// biome-ignore lint/suspicious/noControlCharactersInRegex: rejecting control characters is the point
const CONTROL = /[\u0000-\u001f\u007f]/;
const bad = (m: string): never => {
  throw new MgmtError("bad_request", m);
};
const obj = (v: unknown): Record<string, unknown> => (v && typeof v === "object" && !Array.isArray(v) ? (v as Record<string, unknown>) : {});

function str(v: unknown, field: string, max: number): string | undefined {
  if (v === undefined || v === null || v === "") return undefined;
  if (typeof v !== "string") return bad(`${field} must be text.`);
  const t = v.trim();
  if (t.length > max) return bad(`${field} is too long.`);
  if (CONTROL.test(t)) return bad(`${field} has invalid characters.`);
  return t || undefined;
}
function pubkey(v: unknown): string {
  const p = typeof v === "string" ? v.toLowerCase() : "";
  return HEX64.test(p) ? p : bad("Choose one of your agents.");
}

/** env var values never leave the desktop: keys stay (the UI shows which are set), values are blanked. */
export function blankEnv(e: unknown): Record<string, string> {
  return Object.fromEntries(Object.keys(obj(e)).map((k) => [k, ""]));
}
export function sanitizeAgent(a: unknown): unknown {
  const r = { ...obj(a) };
  if ("env_vars" in r) r.env_vars = blankEnv(r.env_vars);
  if ("system_prompt" in r) r.system_prompt = null;
  if ("private_key_nsec" in r) r.private_key_nsec = "";
  return r;
}
export function sanitizePersona(p: unknown): unknown {
  const r = { ...obj(p) };
  if ("env_vars" in r) r.env_vars = blankEnv(r.env_vars);
  if ("system_prompt" in r) r.system_prompt = "";
  return r;
}
function sanitizeRuntime(e: unknown): unknown {
  const r = { ...obj(e) };
  if ("definition_env" in r) r.definition_env = blankEnv(r.definition_env);
  return r;
}
export function sanitizeGlobal(c: unknown): unknown {
  const r = { ...obj(c) };
  r.env_vars = blankEnv(r.env_vars);
  return r;
}
const list = (v: unknown, f: (x: unknown) => unknown, max = 500) => (Array.isArray(v) ? v.slice(0, max).map(f) : []);

export const RESPOND_MODES = ["owner-only", "anyone", "allowlist", "nobody"] as const;
function respondFields(src: Record<string, unknown>, out: Record<string, unknown>) {
  if (src.respondTo !== undefined) {
    if (typeof src.respondTo !== "string" || !(RESPOND_MODES as readonly string[]).includes(src.respondTo)) bad("Unknown access mode.");
    out.respondTo = src.respondTo;
  }
  if (src.respondToAllowlist !== undefined) {
    const l = src.respondToAllowlist;
    if (!Array.isArray(l) || l.length > 500) return bad("The allow list is too long.");
    out.respondToAllowlist = l.map((x) => (typeof x === "string" && HEX64.test(x.toLowerCase()) ? x.toLowerCase() : bad("The allow list has an invalid key.")));
  }
}

async function runtimeFor(deps: CmdDeps, command: string): Promise<void> {
  const cat = list(await deps.invokeRaw("discover_acp_providers", { force: false }), (x) => obj(x), 200) as Array<Record<string, unknown>>;
  const ok = cat.some((r) => r.command === command && r.availability === "available" && r.source !== "custom");
  if (!ok) throw new MgmtError("runtime_unavailable", "Custom harnesses can't be set remotely. Pick an installed harness, or edit it on the desktop.");
}

export const CMD_READS = [
  "list_managed_agents",
  "list_managed_agent_runtimes",
  "list_personas",
  "discover_acp_providers",
  "agent_access_owner_only",
  "get_global_agent_config",
] as const;
export const CMD_WRITES = ["start_managed_agent", "stop_managed_agent", "create_managed_agent", "update_managed_agent"] as const;
export const CMD_NAMES: readonly string[] = [...CMD_READS, ...CMD_WRITES];

/** The result of a proxied command, or throws MgmtError. `which` tells the caller whether the call mutates agent state. */
export async function handleCmd(name: string, args: unknown, deps: CmdDeps): Promise<unknown> {
  const a = obj(args);
  const inv = deps.invokeRaw;
  switch (name) {
    case "list_managed_agents":
      return list(await inv(name), sanitizeAgent, 200);
    case "list_managed_agent_runtimes":
      return list(await inv(name), (x) => x, 200);
    case "list_personas":
      return list(await inv(name), sanitizePersona, 200);
    case "discover_acp_providers":
      return list(await inv(name, { force: a.force === true }), sanitizeRuntime, 200);
    case "agent_access_owner_only":
      return (await inv(name)) === true;
    case "get_global_agent_config":
      return sanitizeGlobal(await inv(name));
    case "start_managed_agent":
      return sanitizeAgent(await inv(name, { pubkey: pubkey(a.pubkey), expectedRelayUrl: null, expectedSignerPubkey: deps.ownerPubkey ?? null, replayFloorUnix: null }));
    case "stop_managed_agent":
      return sanitizeAgent(await inv(name, { pubkey: pubkey(a.pubkey) }));
    case "create_managed_agent": {
      const i = obj(a.input);
      const out: Record<string, unknown> = { name: str(i.name, "Name", 64) ?? bad("Name is required.") };
      for (const [k, max] of [["personaId", 100], ["provider", 100], ["model", 100]] as const) {
        const v = str(i[k], k, max);
        if (v) out[k] = v;
      }
      if (i.spawnAfterCreate === true) out.spawnAfterCreate = true;
      const cmd = str(i.agentCommand, "Harness", 500);
      if (cmd) {
        await runtimeFor(deps, cmd);
        out.agentCommand = cmd;
        out.harnessOverride = i.harnessOverride === true;
      }
      respondFields(i, out);
      const r = obj(await inv(name, { input: out }));
      return { ...r, agent: sanitizeAgent(r.agent), private_key_nsec: "" };
    }
    case "update_managed_agent": {
      const i = obj(a.input);
      const out: Record<string, unknown> = { pubkey: pubkey(i.pubkey) };
      const nm = str(i.name, "Name", 64);
      if (i.name !== undefined) out.name = nm ?? bad("Name is required.");
      for (const [k, max] of [["provider", 100], ["model", 100]] as const)
        if (k in i) out[k] = str(i[k], k, max) ?? null;
      const cmd = str(i.agentCommand, "Harness", 500);
      if (cmd) {
        await runtimeFor(deps, cmd);
        out.agentCommand = cmd;
        out.harnessOverride = i.harnessOverride === true;
      }
      respondFields(i, out);
      if (Object.keys(out).length === 1) bad("Nothing to change.");
      const r = obj(await inv(name, { input: out }));
      return { ...r, agent: sanitizeAgent(r.agent) };
    }
    default:
      throw new MgmtError("unknown_op", "That command isn't available remotely.");
  }
}
