import type { SessionRecord } from "./types";

const STORAGE_KEY = "buzz.desktop-next.sessions.v1";

type Registry = Record<string, Record<string, SessionRecord>>;

function readRegistry(): Registry {
  try {
    const value = localStorage.getItem(STORAGE_KEY);
    return value ? (JSON.parse(value) as Registry) : {};
  } catch {
    return {};
  }
}

function writeRegistry(registry: Registry) {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(registry));
}

export function listSessions(scope: string): SessionRecord[] {
  return Object.values(readRegistry()[scope] ?? {}).sort(
    (left, right) => right.updatedAt - left.updatedAt,
  );
}

export function rememberSession(scope: string, record: SessionRecord) {
  const registry = readRegistry();
  registry[scope] = { ...(registry[scope] ?? {}), [record.channelId]: record };
  writeRegistry(registry);
}

export function updateSession(
  scope: string,
  channelId: string,
  patch: Partial<SessionRecord>,
) {
  const registry = readRegistry();
  const current = registry[scope]?.[channelId];
  if (!current) return;
  registry[scope] = {
    ...registry[scope],
    [channelId]: { ...current, ...patch, updatedAt: Date.now() },
  };
  writeRegistry(registry);
}

export function forgetSession(scope: string, channelId: string) {
  const registry = readRegistry();
  if (!registry[scope]) return;
  delete registry[scope][channelId];
  writeRegistry(registry);
}
