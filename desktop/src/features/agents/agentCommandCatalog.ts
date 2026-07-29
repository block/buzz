import { normalizePubkey } from "@/shared/lib/pubkey";
import { setLocalStorageItemWithRecovery } from "@/shared/lib/localStorageQuota";

const STORAGE_PREFIX = "buzz-agent-command-catalog.v1";
const MAX_COMMANDS_PER_AGENT = 256;
const MAX_COMMAND_NAME_LENGTH = 128;
const MAX_COMMAND_DESCRIPTION_LENGTH = 512;

export type AgentCommand = {
  name: string;
  description: string | null;
};

export type AgentCommandCatalogEntry = {
  commands: readonly AgentCommand[];
  seq: number;
  timestamp: string;
};

export type AgentCommandCatalog = ReadonlyMap<string, AgentCommandCatalogEntry>;

type PersistedCatalog = {
  version: 1;
  agents: Record<string, AgentCommandCatalogEntry>;
};

type AvailableCommandsEvent = {
  payload: unknown;
  seq: number;
  timestamp: string;
};

const EMPTY_CATALOG: AgentCommandCatalog = new Map();
// Managed-agent observer ingestion is owner-global; command capabilities belong
// to the agent pubkey rather than whichever community currently renders it.
const catalogByOwner = new Map<string, AgentCommandCatalog>();
const hydratedOwners = new Set<string>();
const listeners = new Set<() => void>();

function storageKey(ownerPubkey: string): string {
  return `${STORAGE_PREFIX}:${normalizePubkey(ownerPubkey)}`;
}

function sanitizeCommand(value: unknown): AgentCommand | null {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return null;
  }
  const record = value as Record<string, unknown>;
  if (typeof record.name !== "string") return null;

  const name = record.name.trim().replace(/^\/+/, "");
  if (
    name.length === 0 ||
    name.length > MAX_COMMAND_NAME_LENGTH ||
    /\s|\//u.test(name)
  ) {
    return null;
  }

  const description =
    typeof record.description === "string"
      ? record.description.trim().slice(0, MAX_COMMAND_DESCRIPTION_LENGTH) ||
        null
      : null;
  return { name, description };
}

export function parseAvailableCommandsPayload(
  payload: unknown,
): readonly AgentCommand[] | null {
  if (
    typeof payload !== "object" ||
    payload === null ||
    Array.isArray(payload)
  ) {
    return null;
  }
  const commands = (payload as Record<string, unknown>).commands;
  if (!Array.isArray(commands)) return null;

  const parsed: AgentCommand[] = [];
  const seen = new Set<string>();
  for (const value of commands.slice(0, MAX_COMMANDS_PER_AGENT)) {
    const command = sanitizeCommand(value);
    if (!command) continue;
    const key = command.name.toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    parsed.push(command);
  }
  return parsed;
}

function parseStoredCatalog(raw: string | null): AgentCommandCatalog {
  if (!raw) return EMPTY_CATALOG;
  try {
    const parsed = JSON.parse(raw) as Partial<PersistedCatalog>;
    if (
      parsed.version !== 1 ||
      !parsed.agents ||
      typeof parsed.agents !== "object"
    ) {
      return EMPTY_CATALOG;
    }
    const next = new Map<string, AgentCommandCatalogEntry>();
    for (const [pubkey, entry] of Object.entries(parsed.agents)) {
      if (!entry || typeof entry !== "object") continue;
      const candidate = entry as Partial<AgentCommandCatalogEntry>;
      const commands = parseAvailableCommandsPayload({
        commands: candidate.commands,
      });
      if (
        commands === null ||
        typeof candidate.seq !== "number" ||
        !Number.isSafeInteger(candidate.seq) ||
        typeof candidate.timestamp !== "string"
      ) {
        continue;
      }
      next.set(normalizePubkey(pubkey), {
        commands,
        seq: candidate.seq,
        timestamp: candidate.timestamp,
      });
    }
    return next;
  } catch {
    return EMPTY_CATALOG;
  }
}

function hydrate(ownerPubkey: string): AgentCommandCatalog {
  const owner = normalizePubkey(ownerPubkey);
  if (!hydratedOwners.has(owner)) {
    const raw =
      typeof window === "undefined"
        ? null
        : window.localStorage.getItem(storageKey(owner));
    catalogByOwner.set(owner, parseStoredCatalog(raw));
    hydratedOwners.add(owner);
  }
  return catalogByOwner.get(owner) ?? EMPTY_CATALOG;
}

function persist(ownerPubkey: string, catalog: AgentCommandCatalog): void {
  if (typeof window === "undefined") return;
  const agents = Object.fromEntries(catalog.entries());
  setLocalStorageItemWithRecovery(
    storageKey(ownerPubkey),
    JSON.stringify({ version: 1, agents } satisfies PersistedCatalog),
  );
}

function isNewer(
  incoming: Pick<AgentCommandCatalogEntry, "seq" | "timestamp">,
  current: AgentCommandCatalogEntry | undefined,
): boolean {
  if (!current) return true;
  const incomingTime = Date.parse(incoming.timestamp);
  const currentTime = Date.parse(current.timestamp);
  if (Number.isFinite(incomingTime) && Number.isFinite(currentTime)) {
    if (incomingTime !== currentTime) return incomingTime > currentTime;
  }
  return incoming.seq > current.seq;
}

export function recordAvailableCommandsUpdate(
  ownerPubkey: string,
  agentPubkey: string,
  event: AvailableCommandsEvent,
): boolean {
  const commands = parseAvailableCommandsPayload(event.payload);
  if (commands === null || !Number.isSafeInteger(event.seq)) return false;

  const owner = normalizePubkey(ownerPubkey);
  const agent = normalizePubkey(agentPubkey);
  const current = hydrate(owner);
  if (!isNewer(event, current.get(agent))) return false;

  const next = new Map(current);
  next.set(agent, {
    commands,
    seq: event.seq,
    timestamp: event.timestamp,
  });
  catalogByOwner.set(owner, next);
  persist(owner, next);
  for (const listener of listeners) listener();
  return true;
}

export function getAgentCommandCatalog(
  ownerPubkey: string | null,
): AgentCommandCatalog {
  return ownerPubkey ? hydrate(ownerPubkey) : EMPTY_CATALOG;
}

export function subscribeAgentCommandCatalog(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function resetAgentCommandCatalogForTests(): void {
  catalogByOwner.clear();
  hydratedOwners.clear();
  listeners.clear();
}
