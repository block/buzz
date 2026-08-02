import type { ObserverEvent } from "../ui/agentSessionTypes";

export type AgentAvailableCommand = {
  name: string;
  description: string | null;
  inputHint: string | null;
};

type UnknownRecord = Record<string, unknown>;

function asRecord(value: unknown): UnknownRecord | null {
  return typeof value === "object" && value !== null
    ? (value as UnknownRecord)
    : null;
}

function optionalString(value: unknown): string | null {
  if (typeof value !== "string") return null;
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

function normalizeCommand(value: unknown): AgentAvailableCommand | null {
  if (typeof value === "string") {
    const name = value.trim().replace(/^\/+/, "");
    return name && !/\s/.test(name)
      ? { name, description: null, inputHint: null }
      : null;
  }

  const record = asRecord(value);
  if (!record) return null;

  const name = optionalString(record.name)?.replace(/^\/+/, "") ?? "";
  if (!name || /\s/.test(name)) return null;

  return {
    name,
    description: optionalString(record.description),
    inputHint:
      optionalString(record.inputHint) ?? optionalString(record.input_hint),
  };
}

export function normalizeAvailableCommands(
  value: unknown,
): AgentAvailableCommand[] {
  if (!Array.isArray(value)) return [];

  const commands: AgentAvailableCommand[] = [];
  const seen = new Set<string>();
  for (const candidate of value) {
    const command = normalizeCommand(candidate);
    if (!command) continue;
    const key = command.name.toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    commands.push(command);
  }
  return commands;
}

/**
 * Parse the ACP command catalog advertised by an agent runtime. `null` means
 * the observer event was not a command-catalog update; an empty array is a
 * valid update that clears a previously advertised catalog.
 */
export function parseAvailableCommandsEvent(
  event: ObserverEvent,
): AgentAvailableCommand[] | null {
  if (event.kind !== "acp_read") return null;

  const payload = asRecord(event.payload);
  if (payload?.method !== "session/update") return null;
  const params = asRecord(payload.params);
  const update = asRecord(params?.update);
  if (update?.sessionUpdate !== "available_commands_update") return null;

  return normalizeAvailableCommands(update.availableCommands);
}
