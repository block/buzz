import type { ExtractedPlanningDocument } from "@/shared/api/tauriBattleRhythm";
import type { ConstraintSeverity, PlanningProject } from "./contracts";

export type ProposedPlanningProject = Readonly<{
  id: string;
  title: string;
  purpose: string;
  missionReadyDate: string;
  owner: string;
  sourceEvidence: string;
}>;

export type ProposedPlanningTask = Readonly<{
  wbs: string;
  title: string;
  owner: string;
  plannedStart: string;
  dueDate: string;
  durationWorkdays: number;
  percentComplete: number;
  dependencyWbs: readonly string[];
  sourceLocation: string;
}>;

export type ProposedMissionConstraint = Readonly<{
  description: string;
  owner: string;
  severity: ConstraintSeverity;
  requiredDate: string | null;
  linkedTaskWbs: string | null;
  sourceLocation: string;
}>;

export type PlanImportUncertainty = Readonly<{
  location: string;
  message: string;
  blocking: boolean;
}>;

export type PlanImportProposal = Readonly<{
  schemaVersion: 1;
  project: ProposedPlanningProject;
  tasks: readonly ProposedPlanningTask[];
  constraints: readonly ProposedMissionConstraint[];
  uncertainties: readonly PlanImportUncertainty[];
}>;

type Row = Readonly<{ location: string; values: readonly string[] }>;

function fail(message: string): never {
  throw new Error(`Invalid plan import proposal: ${message}`);
}

function exact(
  value: unknown,
  keys: readonly string[],
): Record<string, unknown> {
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

function text(value: unknown, name: string, maximum = 8192): string {
  if (typeof value !== "string" || !value.trim() || value.length > maximum)
    fail(`${name} must be bounded nonempty text`);
  return value;
}

function date(value: unknown, name: string): string {
  const result = text(value, name, 10);
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(result);
  if (!match) fail(`${name} must be YYYY-MM-DD`);
  const parsed = new Date(`${result}T00:00:00Z`);
  if (
    parsed.getUTCFullYear() !== Number(match[1]) ||
    parsed.getUTCMonth() + 1 !== Number(match[2]) ||
    parsed.getUTCDate() !== Number(match[3])
  )
    fail(`${name} must be a real date`);
  return result;
}

function integer(
  value: unknown,
  name: string,
  minimum: number,
  maximum: number,
): number {
  if (
    typeof value !== "number" ||
    !Number.isInteger(value) ||
    value < minimum ||
    value > maximum
  )
    fail(`${name} must be an integer from ${minimum} to ${maximum}`);
  return value;
}

function columnNumber(value: string): number {
  return (
    [...value].reduce(
      (number, character) => number * 26 + character.charCodeAt(0) - 64,
      0,
    ) - 1
  );
}

function extractedRows(document: ExtractedPlanningDocument): Row[] {
  const rows: Row[] = [];
  const spreadsheet = new Map<
    string,
    { location: string; values: Map<number, string> }
  >();
  for (const block of document.blocks) {
    if (block.kind === "table_row") {
      rows.push({ location: block.location, values: block.cells });
    } else if (block.kind === "spreadsheet_cell") {
      const match = /^([A-Z]+)(\d+)$/.exec(block.coordinate.toUpperCase());
      if (!match) continue;
      const key = `${block.sheet}:${match[2]}`;
      const row = spreadsheet.get(key) ?? {
        location: `${block.sheet}!row ${match[2]}`,
        values: new Map<number, string>(),
      };
      row.values.set(columnNumber(match[1]), block.value);
      spreadsheet.set(key, row);
    } else if (block.kind === "pdf_page") {
      block.text.split(/\r?\n/).forEach((line, index) => {
        const values = line
          .split(/\t|\s{2,}|\s*\|\s*/)
          .map((value) => value.trim());
        if (values.some(Boolean))
          rows.push({
            location: `page ${block.page} line ${index + 1}`,
            values,
          });
      });
    }
  }
  for (const row of spreadsheet.values()) {
    const maximum = Math.max(...row.values.keys());
    rows.push({
      location: row.location,
      values: Array.from(
        { length: maximum + 1 },
        (_, index) => row.values.get(index) ?? "",
      ),
    });
  }
  return rows;
}

const HEADER_ALIASES = new Map([
  ["wbs", "wbs"],
  ["id", "wbs"],
  ["task", "title"],
  ["activity", "title"],
  ["action", "title"],
  ["owner", "owner"],
  ["responsible", "owner"],
  ["oic", "owner"],
  ["start", "plannedStart"],
  ["planned start", "plannedStart"],
  ["due", "dueDate"],
  ["finish", "dueDate"],
  ["end", "dueDate"],
  ["duration", "durationWorkdays"],
  ["duration days", "durationWorkdays"],
  ["progress", "percentComplete"],
  ["percent complete", "percentComplete"],
  ["complete", "percentComplete"],
  ["dependencies", "dependencyWbs"],
  ["dependency", "dependencyWbs"],
  ["predecessors", "dependencyWbs"],
]);

function normalizeHeader(value: string): string {
  return value
    .trim()
    .toLowerCase()
    .replace(/[%/()_-]+/g, " ")
    .replace(/\s+/g, " ");
}

function headerFor(row: Row): Map<string, number> | null {
  const header = new Map<string, number>();
  row.values.forEach((value, index) => {
    const mapped = HEADER_ALIASES.get(normalizeHeader(value));
    if (mapped) header.set(mapped, index);
  });
  return header.has("wbs") &&
    header.has("title") &&
    header.has("owner") &&
    header.has("plannedStart") &&
    header.has("dueDate") &&
    header.has("durationWorkdays")
    ? header
    : null;
}

function cell(row: Row, header: Map<string, number>, field: string): string {
  const index = header.get(field);
  return index === undefined ? "" : (row.values[index] ?? "").trim();
}

function parsePercent(value: string): number | null {
  if (!value) return 0;
  const parsed = Number(value.replace("%", "").trim());
  return Number.isInteger(parsed) && parsed >= 0 && parsed <= 100
    ? parsed
    : null;
}

function dependencyValues(value: string): string[] {
  if (!value.trim()) return [];
  return value
    .split(/[,;]\s*|\s+/)
    .map((item) => item.trim())
    .filter(Boolean);
}

export function parsePlanImportProposal(value: unknown): PlanImportProposal {
  const root = exact(value, [
    "schemaVersion",
    "project",
    "tasks",
    "constraints",
    "uncertainties",
  ]);
  if (root.schemaVersion !== 1) fail("schemaVersion must be 1");
  const projectValue = exact(root.project, [
    "id",
    "title",
    "purpose",
    "missionReadyDate",
    "owner",
    "sourceEvidence",
  ]);
  const project = Object.freeze({
    id: text(projectValue.id, "project id", 256),
    title: text(projectValue.title, "project title", 512),
    purpose: text(projectValue.purpose, "project purpose"),
    missionReadyDate: date(
      projectValue.missionReadyDate,
      "project missionReadyDate",
    ),
    owner: text(projectValue.owner, "project owner", 512),
    sourceEvidence: text(projectValue.sourceEvidence, "project sourceEvidence"),
  });
  if (!Array.isArray(root.tasks) || root.tasks.length > 10_000)
    fail("tasks must be a bounded array");
  const tasks = root.tasks.map((value) => {
    const task = exact(value, [
      "wbs",
      "title",
      "owner",
      "plannedStart",
      "dueDate",
      "durationWorkdays",
      "percentComplete",
      "dependencyWbs",
      "sourceLocation",
    ]);
    if (
      !Array.isArray(task.dependencyWbs) ||
      task.dependencyWbs.some(
        (dependency) => typeof dependency !== "string" || !dependency.trim(),
      )
    )
      fail("dependencyWbs must be a text array");
    return Object.freeze({
      wbs: text(task.wbs, "task WBS", 128),
      title: text(task.title, "task title", 512),
      owner: text(task.owner, "task owner", 512),
      plannedStart: date(task.plannedStart, "task plannedStart"),
      dueDate: date(task.dueDate, "task dueDate"),
      durationWorkdays: integer(
        task.durationWorkdays,
        "task durationWorkdays",
        1,
        3650,
      ),
      percentComplete: integer(
        task.percentComplete,
        "task percentComplete",
        0,
        100,
      ),
      dependencyWbs: Object.freeze([
        ...task.dependencyWbs,
      ]) as readonly string[],
      sourceLocation: text(task.sourceLocation, "task sourceLocation"),
    });
  });
  const wbs = new Set(tasks.map((task) => task.wbs));
  if (wbs.size !== tasks.length) fail("duplicate task WBS");
  for (const task of tasks)
    if (
      task.dependencyWbs.includes(task.wbs) ||
      task.dependencyWbs.some((dependency) => !wbs.has(dependency))
    )
      fail(`task ${task.wbs} has an invalid dependency reference`);
  if (!Array.isArray(root.constraints) || root.constraints.length > 10_000)
    fail("constraints must be a bounded array");
  const constraints = root.constraints.map((value) => {
    const constraint = exact(value, [
      "description",
      "owner",
      "severity",
      "requiredDate",
      "linkedTaskWbs",
      "sourceLocation",
    ]);
    if (
      typeof constraint.severity !== "string" ||
      !["low", "medium", "high", "critical"].includes(constraint.severity)
    )
      fail("invalid constraint severity");
    const linkedTaskWbs =
      constraint.linkedTaskWbs === null
        ? null
        : text(constraint.linkedTaskWbs, "constraint linkedTaskWbs", 128);
    if (linkedTaskWbs !== null && !wbs.has(linkedTaskWbs))
      fail("constraint has an invalid task reference");
    return Object.freeze({
      description: text(constraint.description, "constraint description", 512),
      owner: text(constraint.owner, "constraint owner", 512),
      severity: constraint.severity as ConstraintSeverity,
      requiredDate:
        constraint.requiredDate === null
          ? null
          : date(constraint.requiredDate, "constraint requiredDate"),
      linkedTaskWbs,
      sourceLocation: text(
        constraint.sourceLocation,
        "constraint sourceLocation",
      ),
    });
  });
  if (!Array.isArray(root.uncertainties) || root.uncertainties.length > 10_000)
    fail("uncertainties must be a bounded array");
  const uncertainties = root.uncertainties.map((value) => {
    const uncertainty = exact(value, ["location", "message", "blocking"]);
    if (typeof uncertainty.blocking !== "boolean")
      fail("uncertainty blocking must be boolean");
    return Object.freeze({
      location: text(uncertainty.location, "uncertainty location"),
      message: text(uncertainty.message, "uncertainty message"),
      blocking: uncertainty.blocking,
    });
  });
  return Object.freeze({
    schemaVersion: 1,
    project,
    tasks: Object.freeze(tasks),
    constraints: Object.freeze(constraints),
    uncertainties: Object.freeze(uncertainties),
  });
}

export function interpretPlanDocument(
  document: ExtractedPlanningDocument,
  project: PlanningProject,
): PlanImportProposal {
  const rows = extractedRows(document);
  let header: Map<string, number> | null = null;
  const tasks: ProposedPlanningTask[] = [];
  const uncertainties: PlanImportUncertainty[] = [];
  const seen = new Set<string>();
  for (const row of rows) {
    const candidateHeader = headerFor(row);
    if (candidateHeader) {
      header = candidateHeader;
      continue;
    }
    if (!header || row.values.every((value) => !value.trim())) continue;
    const wbs = cell(row, header, "wbs");
    const title = cell(row, header, "title");
    if (!wbs && !title) continue;
    if (seen.has(wbs)) {
      uncertainties.push({
        location: row.location,
        message: `Duplicate WBS ${wbs} was not imported.`,
        blocking: true,
      });
      continue;
    }
    const duration = Number(cell(row, header, "durationWorkdays"));
    const progress = parsePercent(cell(row, header, "percentComplete"));
    const start = cell(row, header, "plannedStart");
    const due = cell(row, header, "dueDate");
    try {
      const task = {
        wbs: text(wbs, "task WBS", 128),
        title: text(title, "task title", 512),
        owner: text(cell(row, header, "owner"), "task owner", 512),
        plannedStart: date(start, "task plannedStart"),
        dueDate: date(due, "task dueDate"),
        durationWorkdays: integer(duration, "task durationWorkdays", 1, 3650),
        percentComplete:
          progress === null
            ? fail("task percentComplete must be an integer from 0 to 100")
            : progress,
        dependencyWbs: dependencyValues(cell(row, header, "dependencyWbs")),
        sourceLocation: row.location,
      };
      seen.add(wbs);
      tasks.push(task);
    } catch (cause) {
      uncertainties.push({
        location: row.location,
        message:
          cause instanceof Error
            ? cause.message
            : "Planning row is invalid and was not imported.",
        blocking: true,
      });
    }
  }
  for (let index = 0; index < tasks.length; index += 1) {
    const task = tasks[index];
    const missing = task.dependencyWbs.filter(
      (dependency) => !seen.has(dependency),
    );
    if (!missing.length) continue;
    uncertainties.push({
      location: task.sourceLocation,
      message: `Unknown dependency ${missing.join(", ")} was not linked.`,
      blocking: true,
    });
    tasks[index] = {
      ...task,
      dependencyWbs: task.dependencyWbs.filter((dependency) =>
        seen.has(dependency),
      ),
    };
  }
  if (!header)
    uncertainties.push({
      location: document.filename,
      message:
        "No WBS, task, owner, start, due, and duration header row was found.",
      blocking: true,
    });
  return parsePlanImportProposal({
    schemaVersion: 1,
    project: {
      id: project.id,
      title: project.title,
      purpose: project.purpose,
      missionReadyDate: project.missionReadyDate,
      owner: project.owner,
      sourceEvidence: `local-document:${document.sha256}`,
    },
    tasks,
    constraints: [],
    uncertainties,
  });
}
