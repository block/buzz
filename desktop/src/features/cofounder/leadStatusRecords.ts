import { KIND_COFOUNDER_LEAD_HEAD } from "@/shared/constants/kinds";
import type { RelayEvent } from "@/shared/api/types";

import {
  hasOnlyKeys,
  isObject,
  isValidRelayHead,
  oneTagValue,
  parseJson,
  pick,
  readNumber,
} from "./cofounderRecords";

export { KIND_COFOUNDER_LEAD_HEAD } from "@/shared/constants/kinds";

export const COFOUNDER_LEAD_D_TAG = "company:cofounder-lead";

export type CofounderLeadProblem =
  | "no_scout"
  | "several_scouts"
  | "delivery_failed";

export type CofounderLeadStatus = {
  state: "reached" | "unreachable";
  problem: CofounderLeadProblem | null;
  detail: string;
  attempts: number;
  since: number;
  updatedAt: number;
  eventId: string;
};

const LEAD_PROBLEMS = new Set<CofounderLeadProblem>([
  "no_scout",
  "several_scouts",
  "delivery_failed",
]);

export function parseCofounderLeadStatus(
  event: RelayEvent,
  relaySelfPubkey: string,
): CofounderLeadStatus | null {
  if (
    !isValidRelayHead(event, relaySelfPubkey, KIND_COFOUNDER_LEAD_HEAD) ||
    oneTagValue(event, "d") !== COFOUNDER_LEAD_D_TAG
  ) {
    return null;
  }

  const content = parseJson(event.content);
  if (
    !isObject(content) ||
    !hasOnlyKeys(content, [
      "schemaVersion",
      "state",
      "problem",
      "detail",
      "attempts",
      "since",
      "updatedAt",
    ])
  ) {
    return null;
  }

  const state = pick(content, "state");
  const rawProblem = pick(content, "problem");
  const problem = rawProblem === undefined ? null : rawProblem;
  const detail = pick(content, "detail");
  const attempts = readNumber(content, "attempts");
  const since = readNumber(content, "since");
  const updatedAt = readNumber(content, "updatedAt");
  if (
    pick(content, "schemaVersion") !== 1 ||
    (state !== "reached" && state !== "unreachable") ||
    (problem !== null &&
      (typeof problem !== "string" ||
        !LEAD_PROBLEMS.has(problem as CofounderLeadProblem))) ||
    (state === "unreachable") !== (problem !== null) ||
    typeof detail !== "string" ||
    [...detail].length > 200 ||
    attempts === null ||
    attempts > 0xffff_ffff ||
    since === null ||
    updatedAt === null
  ) {
    return null;
  }

  return {
    state,
    problem: problem as CofounderLeadProblem | null,
    detail,
    attempts,
    since,
    updatedAt,
    eventId: event.id,
  };
}
