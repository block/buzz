import type { PlanningTask } from "./contracts";

export type PlanningDepartment = "XO" | "MEO" | "WEEO" | "SO" | string;
export type TaskExecutionMode = "manual" | "scheduled" | "hybrid";
export type TaskOutputType = "response" | "docx" | "pptx" | "xlsx" | "pdf";
export type PlaybookStatus = "active" | "retired";
export type PlaybookTiming = "before" | "after";
export type TaskExecutionStatus = "queued" | "running" | "forReview" | "failed";
export type ArtifactStorageState = "icloud" | "local_pending_icloud";

export type PlanningTaskDetailsV1 = Readonly<{
  schemaVersion: 1;
  id: string;
  projectId: string;
  taskId: string;
  department: PlanningDepartment;
  position: string;
  individual: string | null;
  agentId: string | null;
  dueTime: string | null;
  executionMode: TaskExecutionMode;
  outputType: TaskOutputType;
  playbookId: string | null;
  playbookRevisionId: string | null;
  locked: boolean;
  createdAt: string;
  updatedAt: string;
}>;

export type PlaybookTaskTemplateV1 = Readonly<{
  id: string;
  title: string;
  instructions: string;
  timing: PlaybookTiming;
  offsetMinutes: number;
  durationMinutes: number;
  dependencyIds: readonly string[];
  department: PlanningDepartment;
  position: string;
  agentId: string | null;
  outputType: TaskOutputType;
  reschedulable: boolean;
  locked: boolean;
  linkedCapabilityId: string | null;
  linkedMissionRequirementId: string | null;
}>;

export type PlanningPlaybookV1 = Readonly<{
  schemaVersion: 1;
  id: string;
  title: string;
  description: string;
  status: PlaybookStatus;
  revisionId: string;
  taskTemplates: readonly PlaybookTaskTemplateV1[];
  createdAt: string;
  updatedAt: string;
}>;

export type PlanningTaskExecutionV1 = Readonly<{
  schemaVersion: 1;
  id: string;
  projectId: string;
  taskId: string;
  status: TaskExecutionStatus;
  mode: TaskExecutionMode;
  summary: string | null;
  body: string | null;
  missingInputs: readonly string[];
  assumptions: readonly string[];
  provider: string | null;
  model: string | null;
  startedAt: string;
  completedAt: string | null;
  error: string | null;
  lateStart: boolean;
}>;

export type PlanningTaskArtifactV1 = Readonly<{
  schemaVersion: 1;
  id: string;
  projectId: string;
  taskId: string;
  executionId: string;
  fileName: string;
  path: string;
  format: Exclude<TaskOutputType, "response">;
  storageState: ArtifactStorageState;
  agentId: string | null;
  provider: string | null;
  model: string | null;
  summary: string;
  missingInputWarning: string | null;
  sha256: string;
  sizeBytes: number;
  createdAt: string;
}>;

const executionModes = new Set<TaskExecutionMode>([
  "manual",
  "scheduled",
  "hybrid",
]);
const outputTypes = new Set<TaskOutputType>([
  "response",
  "docx",
  "pptx",
  "xlsx",
  "pdf",
]);
const artifactFormats = new Set<PlanningTaskArtifactV1["format"]>([
  "docx",
  "pptx",
  "xlsx",
  "pdf",
]);
const playbookStatuses = new Set<PlaybookStatus>(["active", "retired"]);
const playbookTimings = new Set<PlaybookTiming>(["before", "after"]);
const executionStatuses = new Set<TaskExecutionStatus>([
  "queued",
  "running",
  "forReview",
  "failed",
]);
const storageStates = new Set<ArtifactStorageState>([
  "icloud",
  "local_pending_icloud",
]);

function fail(message: string): never {
  throw new Error(`Invalid project execution contract: ${message}`);
}

function exactObject(value: unknown, keys: readonly string[]) {
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

function schemaVersion(value: unknown): 1 {
  if (value !== 1) fail("schemaVersion must be 1");
  return 1;
}

function text(value: unknown, name: string, max = 8192): string {
  if (
    typeof value !== "string" ||
    value.trim().length === 0 ||
    value.length > max
  )
    fail(`${name} must be bounded nonempty text`);
  return value;
}

function nullableText(value: unknown, name: string, max = 8192): string | null {
  return value === null ? null : text(value, name, max);
}

function timestamp(value: unknown, name: string): string {
  const result = text(value, name, 64);
  if (Number.isNaN(Date.parse(result)) || !/[zZ]|[+-]\d\d:\d\d$/.test(result))
    fail(`${name} must be RFC3339`);
  return result;
}

function nullableTimestamp(value: unknown, name: string): string | null {
  return value === null ? null : timestamp(value, name);
}

function bool(value: unknown, name: string): boolean {
  if (typeof value !== "boolean") fail(`${name} must be boolean`);
  return value;
}

function integer(
  value: unknown,
  name: string,
  min: number,
  max: number,
): number {
  if (!Number.isInteger(value) || Number(value) < min || Number(value) > max)
    fail(`${name} must be an integer from ${min} to ${max}`);
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

function strings(
  value: unknown,
  name: string,
  maxItems = 128,
): readonly string[] {
  if (
    !Array.isArray(value) ||
    value.length > maxItems ||
    value.some(
      (item) =>
        typeof item !== "string" ||
        item.trim().length === 0 ||
        item.length > 2048,
    )
  )
    fail(`${name} must be a bounded text array`);
  return Object.freeze([...value]) as readonly string[];
}

function dueTime(value: unknown): string | null {
  if (value === null) return null;
  const result = text(value, "dueTime", 5);
  if (!/^(?:[01]\d|2[0-3]):[0-5]\d$/.test(result))
    fail("dueTime must be HH:mm");
  return result;
}

function parseTemplate(value: unknown): PlaybookTaskTemplateV1 {
  const object = exactObject(value, [
    "id",
    "title",
    "instructions",
    "timing",
    "offsetMinutes",
    "durationMinutes",
    "dependencyIds",
    "department",
    "position",
    "agentId",
    "outputType",
    "reschedulable",
    "locked",
    "linkedCapabilityId",
    "linkedMissionRequirementId",
  ]);
  const id = text(object.id, "template id", 256);
  const dependencyIds = strings(object.dependencyIds, "dependencyIds");
  if (dependencyIds.includes(id)) fail("template cannot depend on itself");
  return Object.freeze({
    id,
    title: text(object.title, "template title", 512),
    instructions: text(object.instructions, "template instructions"),
    timing: enumValue(object.timing, playbookTimings, "playbook timing"),
    offsetMinutes: integer(object.offsetMinutes, "offsetMinutes", 0, 5_256_000),
    durationMinutes: integer(
      object.durationMinutes,
      "durationMinutes",
      1,
      525_600,
    ),
    dependencyIds,
    department: text(object.department, "department", 256),
    position: text(object.position, "position", 512),
    agentId: nullableText(object.agentId, "agentId", 256),
    outputType: enumValue(object.outputType, outputTypes, "output type"),
    reschedulable: bool(object.reschedulable, "reschedulable"),
    locked: bool(object.locked, "locked"),
    linkedCapabilityId: nullableText(
      object.linkedCapabilityId,
      "linkedCapabilityId",
      256,
    ),
    linkedMissionRequirementId: nullableText(
      object.linkedMissionRequirementId,
      "linkedMissionRequirementId",
      256,
    ),
  });
}

export function parsePlanningTaskDetails(
  value: unknown,
): PlanningTaskDetailsV1 {
  const object = exactObject(value, [
    "schemaVersion",
    "id",
    "projectId",
    "taskId",
    "department",
    "position",
    "individual",
    "agentId",
    "dueTime",
    "executionMode",
    "outputType",
    "playbookId",
    "playbookRevisionId",
    "locked",
    "createdAt",
    "updatedAt",
  ]);
  return Object.freeze({
    schemaVersion: schemaVersion(object.schemaVersion),
    id: text(object.id, "id", 256),
    projectId: text(object.projectId, "projectId", 256),
    taskId: text(object.taskId, "taskId", 256),
    department: text(object.department, "department", 256),
    position: text(object.position, "position", 512),
    individual: nullableText(object.individual, "individual", 512),
    agentId: nullableText(object.agentId, "agentId", 256),
    dueTime: dueTime(object.dueTime),
    executionMode: enumValue(
      object.executionMode,
      executionModes,
      "execution mode",
    ),
    outputType: enumValue(object.outputType, outputTypes, "output type"),
    playbookId: nullableText(object.playbookId, "playbookId", 256),
    playbookRevisionId: nullableText(
      object.playbookRevisionId,
      "playbookRevisionId",
      256,
    ),
    locked: bool(object.locked, "locked"),
    createdAt: timestamp(object.createdAt, "createdAt"),
    updatedAt: timestamp(object.updatedAt, "updatedAt"),
  });
}

export function parsePlanningPlaybook(value: unknown): PlanningPlaybookV1 {
  const object = exactObject(value, [
    "schemaVersion",
    "id",
    "title",
    "description",
    "status",
    "revisionId",
    "taskTemplates",
    "createdAt",
    "updatedAt",
  ]);
  if (!Array.isArray(object.taskTemplates) || object.taskTemplates.length > 256)
    fail("taskTemplates must be a bounded array");
  const taskTemplates = object.taskTemplates.map(parseTemplate);
  const ids = new Set(taskTemplates.map((template) => template.id));
  if (ids.size !== taskTemplates.length) fail("template IDs must be unique");
  if (
    taskTemplates.some((template) =>
      template.dependencyIds.some((dependencyId) => !ids.has(dependencyId)),
    )
  )
    fail("template dependency must reference the same playbook");
  return Object.freeze({
    schemaVersion: schemaVersion(object.schemaVersion),
    id: text(object.id, "id", 256),
    title: text(object.title, "title", 512),
    description: text(object.description, "description"),
    status: enumValue(object.status, playbookStatuses, "playbook status"),
    revisionId: text(object.revisionId, "revisionId", 256),
    taskTemplates: Object.freeze(taskTemplates),
    createdAt: timestamp(object.createdAt, "createdAt"),
    updatedAt: timestamp(object.updatedAt, "updatedAt"),
  });
}

export function parsePlanningTaskExecution(
  value: unknown,
): PlanningTaskExecutionV1 {
  const object = exactObject(value, [
    "schemaVersion",
    "id",
    "projectId",
    "taskId",
    "status",
    "mode",
    "summary",
    "body",
    "missingInputs",
    "assumptions",
    "provider",
    "model",
    "startedAt",
    "completedAt",
    "error",
    "lateStart",
  ]);
  return Object.freeze({
    schemaVersion: schemaVersion(object.schemaVersion),
    id: text(object.id, "id", 256),
    projectId: text(object.projectId, "projectId", 256),
    taskId: text(object.taskId, "taskId", 256),
    status: enumValue(object.status, executionStatuses, "execution status"),
    mode: enumValue(object.mode, executionModes, "execution mode"),
    summary: nullableText(object.summary, "summary"),
    body: nullableText(object.body, "body", 262_144),
    missingInputs: strings(object.missingInputs, "missingInputs"),
    assumptions: strings(object.assumptions, "assumptions"),
    provider: nullableText(object.provider, "provider", 256),
    model: nullableText(object.model, "model", 256),
    startedAt: timestamp(object.startedAt, "startedAt"),
    completedAt: nullableTimestamp(object.completedAt, "completedAt"),
    error: nullableText(object.error, "error"),
    lateStart: bool(object.lateStart, "lateStart"),
  });
}

export function parsePlanningTaskArtifact(
  value: unknown,
): PlanningTaskArtifactV1 {
  const object = exactObject(value, [
    "schemaVersion",
    "id",
    "projectId",
    "taskId",
    "executionId",
    "fileName",
    "path",
    "format",
    "storageState",
    "agentId",
    "provider",
    "model",
    "summary",
    "missingInputWarning",
    "sha256",
    "sizeBytes",
    "createdAt",
  ]);
  const path = text(object.path, "path", 4096);
  if (!path.startsWith("/")) fail("path must be absolute");
  const sha256 = text(object.sha256, "sha256", 64);
  if (!/^[0-9a-f]{64}$/.test(sha256)) fail("sha256 must be lowercase hex");
  return Object.freeze({
    schemaVersion: schemaVersion(object.schemaVersion),
    id: text(object.id, "id", 256),
    projectId: text(object.projectId, "projectId", 256),
    taskId: text(object.taskId, "taskId", 256),
    executionId: text(object.executionId, "executionId", 256),
    fileName: text(object.fileName, "fileName", 512),
    path,
    format: enumValue(object.format, artifactFormats, "artifact format"),
    storageState: enumValue(
      object.storageState,
      storageStates,
      "storage state",
    ),
    agentId: nullableText(object.agentId, "agentId", 256),
    provider: nullableText(object.provider, "provider", 256),
    model: nullableText(object.model, "model", 256),
    summary: text(object.summary, "summary"),
    missingInputWarning: nullableText(
      object.missingInputWarning,
      "missingInputWarning",
    ),
    sha256,
    sizeBytes: integer(object.sizeBytes, "sizeBytes", 0, 25 * 1024 * 1024),
    createdAt: timestamp(object.createdAt, "createdAt"),
  });
}

export function defaultTaskDetails(task: PlanningTask): PlanningTaskDetailsV1 {
  return Object.freeze({
    schemaVersion: 1,
    id: `details:${task.id}`,
    projectId: task.projectId,
    taskId: task.id,
    department: task.owner,
    position: task.owner,
    individual: null,
    agentId: null,
    dueTime: null,
    executionMode: "manual",
    outputType: "response",
    playbookId: null,
    playbookRevisionId: null,
    locked: false,
    createdAt: task.createdAt,
    updatedAt: task.updatedAt,
  });
}
