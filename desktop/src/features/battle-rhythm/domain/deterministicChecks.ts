import type { BattleRhythmEvent, BattleRhythmSource } from "./contracts";
import { localDateTimeToRfc3339 } from "./dateRange";

export type PlanningFindingCategory =
  | "sourceConflict"
  | "missingPrerequisite"
  | "suspiciousTiming"
  | "possibleOmission"
  | "unresolvedAmbiguity";

export type ProposedCalendarEvent = Readonly<{
  title: string;
  type: string;
  start: string;
  end: string;
  allDay: boolean;
  responsibleOwner: string | null;
  remarks: string | null;
}>;

export type PlanningFinding = Readonly<{
  id: string;
  category: PlanningFindingCategory;
  severity: "info" | "warning" | "critical";
  title: string;
  rationale: string;
  confidence: number;
  evidence: readonly string[];
  affectedEventIds: readonly string[];
  proposedEvent: ProposedCalendarEvent | null;
}>;

function normalize(value: string): string {
  return value
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, " ")
    .trim();
}

function addDays(day: string, amount: number): string {
  const [year, month, date] = day.split("-").map(Number);
  return new Date(Date.UTC(year, month - 1, date + amount))
    .toISOString()
    .slice(0, 10);
}

function weekday(day: string): number {
  const [year, month, date] = day.split("-").map(Number);
  return new Date(Date.UTC(year, month - 1, date)).getUTCDay();
}

function priorWorkingDay(day: string): string {
  let candidate = addDays(day, -1);
  while (weekday(candidate) === 0 || weekday(candidate) === 6)
    candidate = addDays(candidate, -1);
  return candidate;
}

function longWeekday(day: string): string {
  return new Intl.DateTimeFormat("en-AU", {
    weekday: "long",
    timeZone: "UTC",
  }).format(new Date(`${day}T12:00:00Z`));
}

function isSailing(event: BattleRhythmEvent): boolean {
  const title = normalize(event.title);
  return (
    event.status === "approved" &&
    /\b(sail|sailing|depart|departure|proceed to sea)\b/.test(title)
  );
}

function isSecuringForSea(event: BattleRhythmEvent): boolean {
  const title = normalize(event.title);
  return (
    event.status === "approved" &&
    title.includes("securing for sea") &&
    (title.includes("round") || title === "securing for sea")
  );
}

export function evaluatePlanningChecks(
  input: Readonly<{
    events: readonly BattleRhythmEvent[];
    sources: readonly Pick<BattleRhythmSource, "id" | "type">[];
    timeZone: string;
  }>,
): readonly PlanningFinding[] {
  const findings: PlanningFinding[] = [];
  for (const sailing of input.events.filter(isSailing)) {
    const sailingDay = sailing.start.slice(0, 10);
    const requiredDay = priorWorkingDay(sailingDay);
    const prerequisiteExists = input.events.some(
      (event) =>
        isSecuringForSea(event) && event.start.slice(0, 10) === requiredDay,
    );
    if (prerequisiteExists) continue;
    const nextDay = addDays(requiredDay, 1);
    findings.push(
      Object.freeze({
        id: `missing-prerequisite:${sailing.id}:securing-for-sea`,
        category: "missingPrerequisite",
        severity: "warning",
        title: "Securing-for-sea activity may be missing",
        rationale: `${sailing.title} is scheduled on ${longWeekday(sailingDay)}, but no approved securing-for-sea rounds are scheduled on the prior working day (${longWeekday(requiredDay)} ${requiredDay}).`,
        confidence: 0.95,
        evidence: Object.freeze([sailing.id]),
        affectedEventIds: Object.freeze([sailing.id]),
        proposedEvent: Object.freeze({
          title: "Securing for sea rounds",
          type: "readiness",
          start: localDateTimeToRfc3339(`${requiredDay}T00:00`, input.timeZone),
          end: localDateTimeToRfc3339(`${nextDay}T00:00`, input.timeZone),
          allDay: true,
          responsibleOwner: "Executive Officer",
          remarks: `Proposed prerequisite for ${sailing.title} on ${sailingDay}.`,
        }),
      }),
    );
  }

  const sourceType = new Map(
    input.sources.map((source) => [source.id, source.type]),
  );
  const sourceEvents = input.events.filter(
    (event) =>
      event.status === "approved" &&
      event.ownership.kind === "source" &&
      ["fas", "longcast", "shortcast"].includes(
        sourceType.get(event.ownership.sourceId) ?? "",
      ),
  );
  for (let leftIndex = 0; leftIndex < sourceEvents.length; leftIndex += 1) {
    const left = sourceEvents[leftIndex];
    for (
      let rightIndex = leftIndex + 1;
      rightIndex < sourceEvents.length;
      rightIndex += 1
    ) {
      const right = sourceEvents[rightIndex];
      if (
        left.ownership.kind !== "source" ||
        right.ownership.kind !== "source" ||
        left.ownership.sourceId === right.ownership.sourceId ||
        sourceType.get(left.ownership.sourceId) ===
          sourceType.get(right.ownership.sourceId) ||
        normalize(left.title) !== normalize(right.title)
      )
        continue;
      const leftDay = left.start.slice(0, 10);
      const rightDay = right.start.slice(0, 10);
      const separation = Math.abs(
        (Date.parse(`${leftDay}T00:00:00Z`) -
          Date.parse(`${rightDay}T00:00:00Z`)) /
          86_400_000,
      );
      if (leftDay === rightDay || separation > 7) continue;
      const affected = [left.id, right.id].sort();
      findings.push(
        Object.freeze({
          id: `source-conflict:${affected.join(":")}`,
          category: "sourceConflict",
          severity: "critical",
          title: "Planning sources disagree on activity date",
          rationale: `${left.title} appears on ${longWeekday(leftDay)} ${leftDay} in one source and ${longWeekday(rightDay)} ${rightDay} in another.`,
          confidence: 0.9,
          evidence: Object.freeze([
            `${left.ownership.sourceId}:${left.ownership.sourceLocation}`,
            `${right.ownership.sourceId}:${right.ownership.sourceLocation}`,
          ]),
          affectedEventIds: Object.freeze(affected),
          proposedEvent: null,
        }),
      );
    }
  }
  return Object.freeze(
    findings.sort(
      (left, right) =>
        (left.severity === "critical" ? 0 : 1) -
          (right.severity === "critical" ? 0 : 1) ||
        left.id.localeCompare(right.id),
    ),
  );
}
