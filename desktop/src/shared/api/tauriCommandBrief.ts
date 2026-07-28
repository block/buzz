import {
  parseBriefRunStatus,
  parseBriefSchedule,
  parsePublishedCommandBrief,
  type BriefRunStatus,
  type BriefSchedule,
  type PublishedCommandBrief,
} from "@/features/command-console/domain/briefContracts";
import {
  hasExactKeys,
  isRecord,
} from "@/features/command-console/domain/validation";
import { invokeTauri } from "@/shared/api/tauri";

export type CommandBriefStatusView = {
  readonly classification: "OFFICIAL";
  readonly current: BriefRunStatus | null;
  readonly history: readonly BriefRunStatus[];
};

export type CommandBriefScheduleUpdate = {
  readonly enabled: boolean;
  readonly localTime: string;
  readonly concurrency: 1 | 2;
};

export type ModelRoutingPreference = "cloud_first" | "local_first";

export type WorldMonitorConnection = {
  readonly endpoint: string;
  readonly status:
    | "not_configured"
    | "configured"
    | "connected"
    | "unavailable"
    | "unauthorised"
    | "quota_limited";
  readonly briefUsed: number;
  readonly briefLimit: 25;
  readonly directUsed: number;
  readonly directLimit: 25;
};

const WORLD_MONITOR_STATUSES = new Set([
  "not_configured",
  "configured",
  "connected",
  "unavailable",
  "unauthorised",
  "quota_limited",
]);

export function parseWorldMonitorConnection(
  value: unknown,
): WorldMonitorConnection {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, [
      "endpoint",
      "status",
      "briefUsed",
      "briefLimit",
      "directUsed",
      "directLimit",
    ]) ||
    value.endpoint !== "https://api.worldmonitor.app/mcp" ||
    typeof value.status !== "string" ||
    !WORLD_MONITOR_STATUSES.has(value.status) ||
    !Number.isSafeInteger(value.briefUsed) ||
    (value.briefUsed as number) < 0 ||
    (value.briefUsed as number) > 25 ||
    value.briefLimit !== 25 ||
    !Number.isSafeInteger(value.directUsed) ||
    (value.directUsed as number) < 0 ||
    (value.directUsed as number) > 25 ||
    value.directLimit !== 25
  ) {
    return invalidResponse();
  }
  return Object.freeze({
    endpoint: value.endpoint,
    status: value.status as WorldMonitorConnection["status"],
    briefUsed: value.briefUsed as number,
    briefLimit: 25,
    directUsed: value.directUsed as number,
    directLimit: 25,
  });
}

function parseModelRoutingPreference(value: unknown): ModelRoutingPreference {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, ["preference"]) ||
    (value.preference !== "cloud_first" && value.preference !== "local_first")
  ) {
    return invalidResponse();
  }
  return value.preference;
}

function invalidResponse(): never {
  throw new Error("Command Brief returned an invalid response.");
}

function parseStatusView(value: unknown): CommandBriefStatusView {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, ["classification", "current", "history"]) ||
    value.classification !== "OFFICIAL" ||
    !Array.isArray(value.history) ||
    value.history.length > 32
  ) {
    return invalidResponse();
  }
  const current =
    value.current === null ? null : parseBriefRunStatus(value.current);
  const history = value.history.map(parseBriefRunStatus);
  if (
    (value.current !== null && current === null) ||
    history.some((status) => status === null)
  ) {
    return invalidResponse();
  }
  const statuses = history as BriefRunStatus[];
  if (
    (current === null && statuses.length !== 0) ||
    (current !== null &&
      (statuses.length === 0 ||
        statuses.some(
          (entry, index) =>
            entry.runId !== current.runId ||
            (index > 0 && entry.sequence <= statuses[index - 1].sequence),
        ) ||
        statuses.at(-1)?.sequence !== current.sequence ||
        statuses.at(-1)?.state !== current.state ||
        statuses.at(-1)?.updatedAt !== current.updatedAt))
  ) {
    return invalidResponse();
  }
  return Object.freeze({
    classification: "OFFICIAL",
    current,
    history: Object.freeze(statuses),
  });
}

export async function getCommandBriefStatus(): Promise<CommandBriefStatusView> {
  return parseStatusView(
    await invokeTauri<unknown>("get_command_brief_status"),
  );
}

export async function startCommandBrief(): Promise<BriefRunStatus> {
  const parsed = parseBriefRunStatus(
    await invokeTauri<unknown>("start_command_brief"),
  );
  return parsed ?? invalidResponse();
}

export async function cancelCommandBrief(
  runId: string,
): Promise<BriefRunStatus> {
  const parsed = parseBriefRunStatus(
    await invokeTauri<unknown>("cancel_command_brief", { runId }),
  );
  return parsed ?? invalidResponse();
}

export async function getLatestCommandBrief(): Promise<PublishedCommandBrief | null> {
  const value = await invokeTauri<unknown>("get_latest_command_brief");
  if (value === null) return null;
  return parsePublishedCommandBrief(value) ?? invalidResponse();
}

export async function getCommandBriefSchedule(): Promise<BriefSchedule> {
  const parsed = parseBriefSchedule(
    await invokeTauri<unknown>("get_command_brief_schedule"),
  );
  return parsed ?? invalidResponse();
}

export async function setCommandBriefSchedule(
  update: CommandBriefScheduleUpdate,
): Promise<BriefSchedule> {
  const parsed = parseBriefSchedule(
    await invokeTauri<unknown>("set_command_brief_schedule", { update }),
  );
  return parsed ?? invalidResponse();
}

export async function getModelRoutingPreference(): Promise<ModelRoutingPreference> {
  return parseModelRoutingPreference(
    await invokeTauri<unknown>("get_model_routing_preference"),
  );
}

export async function setModelRoutingPreference(
  preference: ModelRoutingPreference,
): Promise<ModelRoutingPreference> {
  return parseModelRoutingPreference(
    await invokeTauri<unknown>("set_model_routing_preference", { preference }),
  );
}

export async function getWorldMonitorConnection(): Promise<WorldMonitorConnection> {
  return parseWorldMonitorConnection(
    await invokeTauri<unknown>("get_world_monitor_connection"),
  );
}

export async function saveWorldMonitorApiKey(
  apiKey: string,
): Promise<WorldMonitorConnection> {
  return parseWorldMonitorConnection(
    await invokeTauri<unknown>("save_world_monitor_api_key", { apiKey }),
  );
}

export async function removeWorldMonitorApiKey(): Promise<WorldMonitorConnection> {
  return parseWorldMonitorConnection(
    await invokeTauri<unknown>("remove_world_monitor_api_key"),
  );
}

export async function testWorldMonitorConnection(): Promise<WorldMonitorConnection> {
  return parseWorldMonitorConnection(
    await invokeTauri<unknown>("test_world_monitor_connection"),
  );
}
