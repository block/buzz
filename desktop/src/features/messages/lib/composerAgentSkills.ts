import type { ObserverEvent } from "@/features/agents/ui/agentSessionTypes";

export type ComposerAgentSkill = {
  description: string;
  inputHint: string;
  name: string;
};

type UnknownRecord = Record<string, unknown>;

function asRecord(value: unknown): UnknownRecord | null {
  return typeof value === "object" && value !== null
    ? (value as UnknownRecord)
    : null;
}

function asString(value: unknown): string {
  return typeof value === "string" ? value.trim() : "";
}

function normalizeSkill(value: unknown): ComposerAgentSkill | null {
  const record = asRecord(value);
  const rawName = record ? asString(record.name) : asString(value);
  const name = rawName.replace(/^\/+/, "");
  if (!name || /\s/.test(name)) return null;

  return {
    description: record ? asString(record.description) : "",
    inputHint: record ? asString(record.inputHint) : "",
    name,
  };
}

function availableSkillsFromEvent(
  event: ObserverEvent,
): ComposerAgentSkill[] | null {
  const payload = asRecord(event.payload);
  if (payload?.method !== "session/update") return null;
  const params = asRecord(payload.params);
  const update = asRecord(params?.update);
  if (update?.sessionUpdate !== "available_commands_update") return null;
  if (!Array.isArray(update.availableCommands)) return [];

  const seen = new Set<string>();
  const skills: ComposerAgentSkill[] = [];
  for (const value of update.availableCommands) {
    const skill = normalizeSkill(value);
    if (!skill || seen.has(skill.name)) continue;
    seen.add(skill.name);
    skills.push(skill);
  }
  return skills;
}

/**
 * Return the commands most recently advertised by an agent session in a
 * channel. A newer session without a command update intentionally returns an
 * empty list instead of leaking stale commands from the previous session.
 */
export function extractAvailableAgentSkills(
  events: readonly ObserverEvent[],
  channelId: string | null,
): ComposerAgentSkill[] {
  const scopedEvents = events.filter((event) => event.channelId === channelId);
  let latestSessionId: string | null = null;
  for (let index = scopedEvents.length - 1; index >= 0; index -= 1) {
    const sessionId = scopedEvents[index]?.sessionId;
    if (sessionId) {
      latestSessionId = sessionId;
      break;
    }
  }
  const latestSessionTurnIds = latestSessionId
    ? new Set(
        scopedEvents
          .filter((event) => event.sessionId === latestSessionId)
          .map((event) => event.turnId)
          .filter((turnId): turnId is string => turnId !== null),
      )
    : null;

  for (let index = scopedEvents.length - 1; index >= 0; index -= 1) {
    const event = scopedEvents[index];
    if (!event) continue;
    const belongsToLatestSession =
      !latestSessionId ||
      event.sessionId === latestSessionId ||
      (event.sessionId === null &&
        event.turnId !== null &&
        latestSessionTurnIds?.has(event.turnId));
    if (!belongsToLatestSession) continue;
    const skills = availableSkillsFromEvent(event);
    if (skills !== null) return skills;
  }
  return [];
}

export function buildSkillInsertion(
  text: string,
  cursor: number,
  skillName: string,
): {
  insertText: string;
  replaceFromOffset: number;
  replaceToOffset: number;
} | null {
  const name = skillName.trim().replace(/^\/+/, "");
  if (!name || /\s/.test(name)) return null;

  const safeCursor = Math.max(0, Math.min(cursor, text.length));
  const needsLeadingSpace =
    safeCursor > 0 && !/\s/.test(text.charAt(safeCursor - 1));
  const needsTrailingSpace =
    safeCursor === text.length || !/\s/.test(text.charAt(safeCursor));

  return {
    insertText: `${needsLeadingSpace ? " " : ""}/${name}${
      needsTrailingSpace ? " " : ""
    }`,
    replaceFromOffset: safeCursor,
    replaceToOffset: safeCursor,
  };
}
