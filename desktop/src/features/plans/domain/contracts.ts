export type ProjectStatus = "draft" | "active" | "complete" | "cancelled";
export type TaskStatus =
  | "notStarted"
  | "inProgress"
  | "blocked"
  | "forReview"
  | "complete"
  | "cancelled";
export type ConstraintStatus =
  | "open"
  | "mitigated"
  | "resolved"
  | "missionChanged"
  | "oplimCandidate"
  | "riskCandidate";
export type ConstraintType =
  | "defect"
  | "missingCapability"
  | "readiness"
  | "externalDependency"
  | "assumption";
export type ConstraintSeverity = "low" | "medium" | "high" | "critical";

export type PlanningProject = Readonly<{
  schemaVersion: 1;
  id: string;
  title: string;
  purpose: string;
  missionReadyDate: string;
  status: ProjectStatus;
  progressPercent: number;
  owner: string;
  linkedActivityIds: readonly string[];
  assumptions: readonly string[];
  createdAt: string;
  updatedAt: string;
}>;

export type PlanningTask = Readonly<{
  schemaVersion: 1;
  id: string;
  projectId: string;
  wbs: string;
  parentTaskId: string | null;
  title: string;
  owner: string;
  status: TaskStatus;
  percentComplete: number;
  plannedStart: string | null;
  dueDate: string | null;
  durationWorkdays: number | null;
  dependencyIds: readonly string[];
  fixedStart: string | null;
  linkedCapabilityId: string | null;
  linkedMissionRequirementId: string | null;
  notes: string | null;
  sourceEvidence: string | null;
  isSummary: boolean;
  createdAt: string;
  updatedAt: string;
}>;

export type MissionConstraint = Readonly<{
  schemaVersion: 1;
  id: string;
  projectId: string;
  type: ConstraintType;
  description: string;
  owner: string;
  severity: ConstraintSeverity;
  status: ConstraintStatus;
  linkedMissionRequirementId: string | null;
  linkedCapabilityId: string | null;
  linkedTaskId: string | null;
  linkedMilestoneId: string | null;
  requiredDate: string | null;
  dispositionNote: string | null;
  sourceEvidence: string | null;
  createdAt: string;
  updatedAt: string;
}>;

const projectStatuses = new Set<ProjectStatus>([
  "draft",
  "active",
  "complete",
  "cancelled",
]);
const taskStatuses = new Set<TaskStatus>([
  "notStarted",
  "inProgress",
  "blocked",
  "forReview",
  "complete",
  "cancelled",
]);
const constraintStatuses = new Set<ConstraintStatus>([
  "open",
  "mitigated",
  "resolved",
  "missionChanged",
  "oplimCandidate",
  "riskCandidate",
]);
const constraintTypes = new Set<ConstraintType>([
  "defect",
  "missingCapability",
  "readiness",
  "externalDependency",
  "assumption",
]);
const constraintSeverities = new Set<ConstraintSeverity>([
  "low",
  "medium",
  "high",
  "critical",
]);

function fail(message: string): never {
  throw new Error(`Invalid planning contract: ${message}`);
}
function object(value: unknown, keys: readonly string[]) {
  if (!value || typeof value !== "object" || Array.isArray(value))
    fail("object required");
  const record = value as Record<string, unknown>;
  if (
    Object.keys(record).length !== keys.length ||
    Object.keys(record).some((key) => !keys.includes(key))
  )
    fail("unknown or missing field");
  return record;
}
function one(value: unknown): 1 {
  if (value !== 1) fail("schemaVersion must be 1");
  return 1;
}
function text(value: unknown, name: string, max = 8192) {
  if (
    typeof value !== "string" ||
    value.trim().length === 0 ||
    value.length > max
  )
    fail(`${name} must be bounded nonempty text`);
  return value;
}
function nullableText(value: unknown, name: string) {
  return value === null ? null : text(value, name);
}
function strings(value: unknown, name: string) {
  if (
    !Array.isArray(value) ||
    value.length > 2048 ||
    value.some((item) => typeof item !== "string" || !item.trim())
  )
    fail(`${name} must be bounded text array`);
  return Object.freeze([...value]) as readonly string[];
}
function date(value: unknown, name: string) {
  const result = text(value, name, 10);
  if (
    !/^\d{4}-\d{2}-\d{2}$/.test(result) ||
    Number.isNaN(Date.parse(`${result}T00:00:00Z`))
  )
    fail(`${name} must be YYYY-MM-DD`);
  return result;
}
function nullableDate(value: unknown, name: string) {
  return value === null ? null : date(value, name);
}
function timestamp(value: unknown, name: string) {
  const result = text(value, name, 64);
  if (Number.isNaN(Date.parse(result)) || !/[zZ]|[+-]\d\d:\d\d$/.test(result))
    fail(`${name} must be RFC3339`);
  return result;
}
function percent(value: unknown, name: string) {
  if (!Number.isInteger(value) || Number(value) < 0 || Number(value) > 100)
    fail(`${name} must be an integer from 0 to 100`);
  return Number(value);
}
function enumValue<T extends string>(
  value: unknown,
  values: ReadonlySet<T>,
  name: string,
): T {
  if (typeof value !== "string" || !values.has(value as T))
    fail(`invalid ${name}`);
  return value as T;
}

export function parsePlanningProject(value: unknown): PlanningProject {
  const o = object(value, [
    "schemaVersion",
    "id",
    "title",
    "purpose",
    "missionReadyDate",
    "status",
    "progressPercent",
    "owner",
    "linkedActivityIds",
    "assumptions",
    "createdAt",
    "updatedAt",
  ]);
  return Object.freeze({
    schemaVersion: one(o.schemaVersion),
    id: text(o.id, "id", 256),
    title: text(o.title, "title", 512),
    purpose: text(o.purpose, "purpose"),
    missionReadyDate: date(o.missionReadyDate, "missionReadyDate"),
    status: enumValue(o.status, projectStatuses, "project status"),
    progressPercent: percent(o.progressPercent, "progressPercent"),
    owner: text(o.owner, "owner", 512),
    linkedActivityIds: strings(o.linkedActivityIds, "linkedActivityIds"),
    assumptions: strings(o.assumptions, "assumptions"),
    createdAt: timestamp(o.createdAt, "createdAt"),
    updatedAt: timestamp(o.updatedAt, "updatedAt"),
  });
}

export function parsePlanningTask(value: unknown): PlanningTask {
  const o = object(value, [
    "schemaVersion",
    "id",
    "projectId",
    "wbs",
    "parentTaskId",
    "title",
    "owner",
    "status",
    "percentComplete",
    "plannedStart",
    "dueDate",
    "durationWorkdays",
    "dependencyIds",
    "fixedStart",
    "linkedCapabilityId",
    "linkedMissionRequirementId",
    "notes",
    "sourceEvidence",
    "isSummary",
    "createdAt",
    "updatedAt",
  ]);
  const id = text(o.id, "id", 256);
  const dependencies = strings(o.dependencyIds, "dependencyIds");
  const parentTaskId = nullableText(o.parentTaskId, "parentTaskId");
  const status = enumValue(o.status, taskStatuses, "task status");
  const isSummary =
    typeof o.isSummary === "boolean"
      ? o.isSummary
      : fail("isSummary must be boolean");
  const duration =
    o.durationWorkdays === null
      ? null
      : Number.isInteger(o.durationWorkdays) &&
          Number(o.durationWorkdays) > 0 &&
          Number(o.durationWorkdays) <= 3650
        ? Number(o.durationWorkdays)
        : fail("durationWorkdays must be a positive integer");
  if (parentTaskId === id || dependencies.includes(id))
    fail("task cannot reference itself");
  if (
    !isSummary &&
    status !== "complete" &&
    status !== "cancelled" &&
    duration === null
  )
    fail("incomplete leaf task requires duration");
  return Object.freeze({
    schemaVersion: one(o.schemaVersion),
    id,
    projectId: text(o.projectId, "projectId", 256),
    wbs: text(o.wbs, "wbs", 128),
    parentTaskId,
    title: text(o.title, "title", 512),
    owner: text(o.owner, "owner", 512),
    status,
    percentComplete: percent(o.percentComplete, "percentComplete"),
    plannedStart: nullableDate(o.plannedStart, "plannedStart"),
    dueDate: nullableDate(o.dueDate, "dueDate"),
    durationWorkdays: duration,
    dependencyIds: dependencies,
    fixedStart: nullableDate(o.fixedStart, "fixedStart"),
    linkedCapabilityId: nullableText(
      o.linkedCapabilityId,
      "linkedCapabilityId",
    ),
    linkedMissionRequirementId: nullableText(
      o.linkedMissionRequirementId,
      "linkedMissionRequirementId",
    ),
    notes: nullableText(o.notes, "notes"),
    sourceEvidence: nullableText(o.sourceEvidence, "sourceEvidence"),
    isSummary,
    createdAt: timestamp(o.createdAt, "createdAt"),
    updatedAt: timestamp(o.updatedAt, "updatedAt"),
  });
}

export function parseMissionConstraint(value: unknown): MissionConstraint {
  const o = object(value, [
    "schemaVersion",
    "id",
    "projectId",
    "type",
    "description",
    "owner",
    "severity",
    "status",
    "linkedMissionRequirementId",
    "linkedCapabilityId",
    "linkedTaskId",
    "linkedMilestoneId",
    "requiredDate",
    "dispositionNote",
    "sourceEvidence",
    "createdAt",
    "updatedAt",
  ]);
  const status = enumValue(o.status, constraintStatuses, "constraint status");
  const result = Object.freeze({
    schemaVersion: one(o.schemaVersion),
    id: text(o.id, "id", 256),
    projectId: text(o.projectId, "projectId", 256),
    type: enumValue(o.type, constraintTypes, "constraint type"),
    description: text(o.description, "description"),
    owner: text(o.owner, "owner", 512),
    severity: enumValue(
      o.severity,
      constraintSeverities,
      "constraint severity",
    ),
    status,
    linkedMissionRequirementId: nullableText(
      o.linkedMissionRequirementId,
      "linkedMissionRequirementId",
    ),
    linkedCapabilityId: nullableText(
      o.linkedCapabilityId,
      "linkedCapabilityId",
    ),
    linkedTaskId: nullableText(o.linkedTaskId, "linkedTaskId"),
    linkedMilestoneId: nullableText(o.linkedMilestoneId, "linkedMilestoneId"),
    requiredDate: nullableDate(o.requiredDate, "requiredDate"),
    dispositionNote: nullableText(o.dispositionNote, "dispositionNote"),
    sourceEvidence: nullableText(o.sourceEvidence, "sourceEvidence"),
    createdAt: timestamp(o.createdAt, "createdAt"),
    updatedAt: timestamp(o.updatedAt, "updatedAt"),
  });
  if (
    !result.linkedMissionRequirementId &&
    !result.linkedCapabilityId &&
    !result.linkedTaskId &&
    !result.linkedMilestoneId
  )
    fail("constraint requires a mission, capability, task, or milestone link");
  if (
    (status === "oplimCandidate" || status === "riskCandidate") &&
    !result.dispositionNote
  )
    fail("candidate disposition requires a note");
  return result;
}
