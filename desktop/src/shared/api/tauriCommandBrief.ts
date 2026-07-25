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
  return Object.freeze({
    classification: "OFFICIAL",
    current,
    history: Object.freeze(history as BriefRunStatus[]),
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
