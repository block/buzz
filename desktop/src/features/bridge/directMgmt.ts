// Desktop-side executor for agent management requests that arrive over the direct (tailnet) connection.
// The Rust listener authenticates the connection (member-key challenge) and checks "Allow remote agent management";
// this module is the second line: it re-checks the switch, allow-lists the operation, and validates every field before
// anything reaches the real agent APIs. Fields that are not listed here are never forwarded (no commands, args, env vars
// or system prompts can be set remotely). Pure: all IO is injected.
import type {
  AcpRuntimeCatalogEntry,
  CreateManagedAgentInput,
  ManagedAgent,
  UpdateManagedAgentInput,
} from "@/shared/api/types";
import { CMD_MAX_TEXT } from "./protocol";

export class MgmtError extends Error {
  code: string;
  constructor(code: string, message: string) {
    super(message);
    this.code = code;
  }
}

export type MgmtDeps = {
  /** "Allow remote agent management" on this desktop. */
  allowed(): boolean;
  listAgents(): Promise<ManagedAgent[]>;
  startAgent(pubkey: string): Promise<unknown>;
  stopAgent(pubkey: string): Promise<unknown>;
  sendMessage(pubkey: string, channelId: string, text: string): Promise<unknown>;
  createAgent(input: CreateManagedAgentInput): Promise<{ agent: ManagedAgent; spawnError?: string | null; profileSyncError?: string | null }>;
  updateAgent(input: UpdateManagedAgentInput): Promise<{ agent: ManagedAgent; profileSyncError?: string | null }>;
  runtimes(): Promise<AcpRuntimeCatalogEntry[]>;
  personas(): Promise<Array<{ id: string; displayName: string }>>;
};

export const MGMT_OPS = [
  "agents.list",
  "agent.start",
  "agent.stop",
  "agent.message",
  "agent.create",
  "agent.rename",
  "agent.update",
  "providers.list",
  "personas.list",
] as const;
export type MgmtOp = (typeof MGMT_OPS)[number];

const HEX64 = /^[0-9a-f]{64}$/;
// biome-ignore lint/suspicious/noControlCharactersInRegex: rejecting control characters is the point
const CONTROL = /[\u0000-\u001f\u007f]/;

function obj(p: unknown): Record<string, unknown> {
  return p && typeof p === "object" && !Array.isArray(p) ? (p as Record<string, unknown>) : {};
}
function bad(message: string): never {
  throw new MgmtError("bad_request", message);
}
function text(v: unknown, field: string, max: number, required = true): string | undefined {
  if (v === undefined || v === null || v === "") {
    if (required) bad(`${field} is required.`);
    return undefined;
  }
  if (typeof v !== "string") bad(`${field} must be text.`);
  const t = (v as string).trim();
  if (!t && required) bad(`${field} is required.`);
  if (t.length > max) bad(`${field} is too long.`);
  if (CONTROL.test(t)) bad(`${field} has invalid characters.`);
  return t || undefined;
}
function pubkeyOf(p: Record<string, unknown>): string {
  const pk = typeof p.pubkey === "string" ? p.pubkey.toLowerCase() : "";
  if (!HEX64.test(pk)) bad("Choose one of your agents.");
  return pk;
}

export function summarizeAgent(a: ManagedAgent) {
  return {
    pubkey: a.pubkey,
    name: a.name,
    status: a.status,
    runtime: a.runtime ?? undefined,
    model: a.model ?? undefined,
    provider: a.provider ?? undefined,
    personaId: a.personaId ?? undefined,
    pid: a.pid ?? undefined,
    lastStartedAt: a.lastStartedAt ?? undefined,
    lastError: a.lastError ?? undefined,
  };
}

async function runtimeCommand(deps: MgmtDeps, runtimeId: string): Promise<string> {
  const entry = (await deps.runtimes()).find((r) => r.id === runtimeId);
  if (!entry || entry.availability !== "available" || !entry.command)
    throw new MgmtError("runtime_unavailable", "That runtime isn't available on this computer.");
  return entry.command;
}

export async function handleMgmt(op: string, params: unknown, deps: MgmtDeps): Promise<unknown> {
  if (!deps.allowed())
    throw new MgmtError("agents_disabled", 'Turn on "Allow remote agent management" in Settings → Agents on this computer.');
  const p = obj(params);
  const known = async (pubkey: string) => {
    const a = (await deps.listAgents()).find((x) => x.pubkey.toLowerCase() === pubkey);
    if (!a) throw new MgmtError("unknown_agent", "That agent isn't on this computer any more.");
    return a;
  };
  switch (op as MgmtOp) {
    case "agents.list":
      return { agents: (await deps.listAgents()).slice(0, 200).map(summarizeAgent) };
    case "providers.list":
      return {
        runtimes: (await deps.runtimes()).map((r) => ({
          id: r.id,
          label: r.label,
          available: r.availability === "available",
        })),
      };
    case "personas.list":
      return { personas: (await deps.personas()).slice(0, 200).map((x) => ({ id: x.id, name: x.displayName })) };
    case "agent.start": {
      const pk = pubkeyOf(p);
      await known(pk);
      await deps.startAgent(pk);
      return {};
    }
    case "agent.stop": {
      const pk = pubkeyOf(p);
      await known(pk);
      await deps.stopAgent(pk);
      return {};
    }
    case "agent.message": {
      const pk = pubkeyOf(p);
      await known(pk);
      const channelId = text(p.channelId, "channelId", 200) as string;
      const body = text(p.text, "text", CMD_MAX_TEXT) as string;
      await deps.sendMessage(pk, channelId, body);
      return {};
    }
    case "agent.create": {
      const name = text(p.name, "Name", 64) as string;
      const provider = text(p.provider, "Provider", 100, false);
      const model = text(p.model, "Model", 100, false);
      const personaId = text(p.personaId, "Persona", 100, false);
      if (personaId && !(await deps.personas()).some((x) => x.id === personaId)) bad("Unknown persona.");
      const runtimeId = text(p.runtimeId, "Runtime", 100, false);
      const input: CreateManagedAgentInput = {
        name,
        ...(personaId ? { personaId } : {}),
        ...(provider ? { provider } : {}),
        ...(model ? { model } : {}),
        spawnAfterCreate: p.start === true,
      };
      if (runtimeId) {
        input.agentCommand = await runtimeCommand(deps, runtimeId);
        input.harnessOverride = true;
      }
      const r = await deps.createAgent(input);
      return { agent: summarizeAgent(r.agent), spawnError: r.spawnError ?? undefined, profileSyncError: r.profileSyncError ?? undefined };
    }
    case "agent.rename":
    case "agent.update": {
      const pk = pubkeyOf(p);
      const current = await known(pk);
      const input: UpdateManagedAgentInput = { pubkey: pk };
      if (op === "agent.rename") input.name = text(p.name, "Name", 64) as string;
      else {
        if ("name" in p) input.name = text(p.name, "Name", 64) as string;
        // null/"" clears the pin; a string sets it
        if ("provider" in p) input.provider = text(p.provider, "Provider", 100, false) ?? null;
        if ("model" in p) input.model = text(p.model, "Model", 100, false) ?? null;
        const runtimeId = text(p.runtimeId, "Runtime", 100, false);
        if (runtimeId) {
          input.agentCommand = await runtimeCommand(deps, runtimeId);
          input.harnessOverride = true;
        }
        if (Object.keys(input).length === 1) bad("Nothing to change.");
      }
      const r = await deps.updateAgent(input);
      return {
        agent: summarizeAgent(r.agent),
        profileSyncError: r.profileSyncError ?? undefined,
        // a running agent keeps its old settings until it is restarted
        restartNeeded: current.status === "running" && op === "agent.update",
      };
    }
    default:
      throw new MgmtError("unknown_op", "That operation isn't supported.");
  }
}

/** The string sent back to the web as the error ("<code>: <message>"); the web splits it again. */
export function mgmtErrorString(e: unknown): string {
  if (e instanceof MgmtError) return `${e.code}: ${e.message}`;
  return `failed: ${String(e instanceof Error ? e.message : e).slice(0, 200)}`;
}
