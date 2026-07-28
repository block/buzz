import type { ExtractedPlanningDocument } from "@/shared/api/tauriBattleRhythm";
import { localDateTimeToRfc3339 } from "./dateRange";
import {
  parseBattleRhythmEvent,
  parseBattleRhythmRevision,
  type BattleRhythmEvent,
  type BattleRhythmRevision,
  type BattleRhythmSourceType,
} from "./contracts";

export type ProposedBattleRhythmEvent = Readonly<{
  title: string;
  type: string;
  start: string;
  end: string;
  allDay: boolean;
  location: string | null;
  responsibleOwner: string | null;
  participants: readonly string[];
  remarks: string | null;
  sourceLocation: string;
}>;

export type ImportUncertainty = Readonly<{
  location: string;
  message: string;
}>;

export type ImportProposal = Readonly<{
  schemaVersion: 1;
  sourceType: "fas" | "longcast" | "shortcast";
  proposedCoverage: Readonly<{ start: string; end: string }>;
  events: readonly ProposedBattleRhythmEvent[];
  uncertainties: readonly ImportUncertainty[];
}>;

export type ImportDiff = Readonly<{
  added: number;
  changed: number;
  removed: number;
  unchanged: number;
  preserved: number;
}>;

function fail(message = "invalid"): never {
  throw new Error(`Import proposal ${message}`);
}

function exact(value: unknown, keys: readonly string[]) {
  if (!value || typeof value !== "object" || Array.isArray(value))
    fail("must be an object");
  const object = value as Record<string, unknown>;
  if (
    Object.keys(object).length !== keys.length ||
    Object.keys(object).some((key) => !keys.includes(key))
  )
    fail("has unknown or missing fields");
  return object;
}

function text(value: unknown, name: string, maximum = 4096) {
  if (
    typeof value !== "string" ||
    value.trim() !== value ||
    value.length === 0 ||
    value.length > maximum ||
    [...value].some((character) => character === "\0")
  )
    fail(`has invalid ${name}`);
  return value;
}

function optionalText(value: unknown, name: string) {
  return value === null ? null : text(value, name);
}

function timestamp(value: unknown, name: string) {
  const result = text(value, name, 64);
  if (Number.isNaN(Date.parse(result)) || !/[zZ]|[+-]\d\d:\d\d$/.test(result))
    fail(`has invalid ${name}`);
  return result;
}

export function parseImportProposal(value: unknown): ImportProposal {
  const root = exact(value, [
    "schemaVersion",
    "sourceType",
    "proposedCoverage",
    "events",
    "uncertainties",
  ]);
  if (
    root.schemaVersion !== 1 ||
    !["fas", "longcast", "shortcast"].includes(root.sourceType as string)
  )
    fail();
  const coverage = exact(root.proposedCoverage, ["start", "end"]);
  const coverageStart = timestamp(coverage.start, "coverage start");
  const coverageEnd = timestamp(coverage.end, "coverage end");
  if (Date.parse(coverageStart) >= Date.parse(coverageEnd))
    fail("has invalid coverage");
  if (!Array.isArray(root.events) || root.events.length > 2_000)
    fail("has too many events");
  const events = root.events.map((value) => {
    const event = exact(value, [
      "title",
      "type",
      "start",
      "end",
      "allDay",
      "location",
      "responsibleOwner",
      "participants",
      "remarks",
      "sourceLocation",
    ]);
    const start = timestamp(event.start, "event start");
    const end = timestamp(event.end, "event end");
    if (
      Date.parse(start) >= Date.parse(end) ||
      Date.parse(start) < Date.parse(coverageStart) ||
      Date.parse(end) > Date.parse(coverageEnd)
    )
      fail("event is outside coverage");
    if (
      typeof event.allDay !== "boolean" ||
      !Array.isArray(event.participants) ||
      event.participants.length > 64
    )
      fail();
    return Object.freeze({
      title: text(event.title, "title"),
      type: text(event.type, "type", 128),
      start,
      end,
      allDay: event.allDay,
      location: optionalText(event.location, "location"),
      responsibleOwner: optionalText(event.responsibleOwner, "owner"),
      participants: Object.freeze(
        event.participants.map((item) => text(item, "participant")),
      ),
      remarks: optionalText(event.remarks, "remarks"),
      sourceLocation: text(event.sourceLocation, "source location"),
    });
  });
  const sourceLocations = new Set(events.map((event) => event.sourceLocation));
  if (sourceLocations.size !== events.length)
    fail("contains duplicate evidence locations");
  if (!Array.isArray(root.uncertainties) || root.uncertainties.length > 2_000)
    fail("has too many uncertainties");
  const uncertainties = root.uncertainties.map((value) => {
    const item = exact(value, ["location", "message"]);
    return Object.freeze({
      location: text(item.location, "uncertainty location"),
      message: text(item.message, "uncertainty message"),
    });
  });
  return Object.freeze({
    schemaVersion: 1,
    sourceType: root.sourceType as ImportProposal["sourceType"],
    proposedCoverage: Object.freeze({
      start: coverageStart,
      end: coverageEnd,
    }),
    events: Object.freeze(events),
    uncertainties: Object.freeze(uncertainties),
  });
}

type Row = Readonly<{ location: string; values: readonly string[] }>;

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
        values: new Map(),
      };
      row.values.set(columnNumber(match[1]), block.value);
      spreadsheet.set(key, row);
    } else if (block.kind === "pdf_page") {
      block.text.split(/\r?\n/).forEach((line, index) => {
        if (line.trim())
          rows.push({
            location: `page ${block.page} line ${index + 1}`,
            values: [line.trim()],
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

function columnNumber(value: string) {
  return (
    [...value].reduce(
      (number, character) => number * 26 + character.charCodeAt(0) - 64,
      0,
    ) - 1
  );
}

function parseDate(value: string, coverageStart: string): string | null {
  const normalized = value.trim();
  const iso = /\b(\d{4})-(\d{1,2})-(\d{1,2})\b/.exec(normalized);
  const slash = /\b(\d{1,2})[/.](\d{1,2})[/.](\d{2,4})\b/.exec(normalized);
  const named =
    /\b(\d{1,2})\s+(Jan(?:uary)?|Feb(?:ruary)?|Mar(?:ch)?|Apr(?:il)?|May|Jun(?:e)?|Jul(?:y)?|Aug(?:ust)?|Sep(?:tember)?|Oct(?:ober)?|Nov(?:ember)?|Dec(?:ember)?)(?:\s+(\d{4}))?\b/i.exec(
      normalized,
    );
  if (iso)
    return `${iso[1]}-${iso[2].padStart(2, "0")}-${iso[3].padStart(2, "0")}`;
  if (slash) {
    const year = slash[3].length === 2 ? `20${slash[3]}` : slash[3];
    return `${year}-${slash[2].padStart(2, "0")}-${slash[1].padStart(2, "0")}`;
  }
  if (named) {
    const month =
      [
        "jan",
        "feb",
        "mar",
        "apr",
        "may",
        "jun",
        "jul",
        "aug",
        "sep",
        "oct",
        "nov",
        "dec",
      ].indexOf(named[2].slice(0, 3).toLowerCase()) + 1;
    const year = named[3] ?? coverageStart.slice(0, 4);
    return `${year}-${String(month).padStart(2, "0")}-${named[1].padStart(2, "0")}`;
  }
  return null;
}

function parseTime(value: string): string | null {
  const compact = /^\s*([01]?\d|2[0-3])([0-5]\d)\s*$/.exec(value);
  const colon = /\b([01]?\d|2[0-3]):([0-5]\d)\b/.exec(value);
  const match = compact ?? colon;
  return match ? `${match[1].padStart(2, "0")}:${match[2]}` : null;
}

export function interpretExtractedDocument(
  document: ExtractedPlanningDocument,
  sourceType: ImportProposal["sourceType"],
  coverage: Readonly<{ start: string; end: string }>,
  timeZone: string,
): ImportProposal {
  const rows = extractedRows(document);
  const events: ProposedBattleRhythmEvent[] = [];
  const uncertainties: ImportUncertainty[] = [];
  let currentDate: string | null = null;
  for (const row of rows) {
    const values = row.values.map((value) => value.trim()).filter(Boolean);
    if (!values.length) continue;
    const date = values
      .map((value) => parseDate(value, coverage.start))
      .find(Boolean);
    if (date) currentDate = date;
    const time = values.map(parseTime).find(Boolean);
    const titleCandidates = values.filter(
      (value) => !parseDate(value, coverage.start) && !parseTime(value),
    );
    const title = titleCandidates.at(-1);
    if (
      !currentDate ||
      !title ||
      /^(date|time|event|activity|location|remarks)$/i.test(title)
    ) {
      continue;
    }
    const allDay = !time;
    const localStart = `${currentDate}T${time ?? "00:00"}`;
    const start = localDateTimeToRfc3339(localStart, timeZone);
    const end = new Date(
      Date.parse(start) + (allDay ? 24 : 1) * 60 * 60 * 1000,
    ).toISOString();
    if (
      Date.parse(start) < Date.parse(coverage.start) ||
      Date.parse(end) > Date.parse(coverage.end)
    ) {
      uncertainties.push({
        location: row.location,
        message: "Entry falls outside the selected coverage window.",
      });
      continue;
    }
    events.push({
      title,
      type: sourceType === "shortcast" ? "routine" : "activity",
      start,
      end,
      allDay,
      location: null,
      responsibleOwner: null,
      participants: [],
      remarks: null,
      sourceLocation: row.location,
    });
  }
  if (!events.length)
    uncertainties.push({
      location: document.filename,
      message:
        "No dated entries were mapped automatically. Review the extracted content and enter events manually.",
    });
  return parseImportProposal({
    schemaVersion: 1,
    sourceType,
    proposedCoverage: coverage,
    events,
    uncertainties,
  });
}

function eventFromProposal(
  sourceId: string,
  revisionId: string,
  timeZone: string,
  proposed: ProposedBattleRhythmEvent,
  prior?: BattleRhythmEvent,
): BattleRhythmEvent {
  return parseBattleRhythmEvent({
    schemaVersion: 1,
    id: prior?.id ?? `${sourceId}:${stableSuffix(proposed.sourceLocation)}`,
    ownership: {
      kind: "source",
      sourceId,
      revisionId,
      sourceLocation: proposed.sourceLocation,
    },
    title: proposed.title,
    description: null,
    type: proposed.type,
    start: proposed.start,
    end: proposed.end,
    allDay: proposed.allDay,
    timeZone,
    status: "approved",
    location: proposed.location,
    responsibleOwner: proposed.responsibleOwner,
    participants: proposed.participants,
    remarks: proposed.remarks,
    linkedPlanId: null,
    linkedTaskId: null,
    linkedMissionRequirementId: null,
    parentActivityId: null,
    recurrence: null,
    excludedOccurrenceStarts: [],
  });
}

function stableSuffix(value: string) {
  let hash = 2166136261;
  for (const character of value)
    hash = Math.imul(hash ^ character.charCodeAt(0), 16777619);
  return (hash >>> 0).toString(16).padStart(8, "0");
}

function sameContent(left: BattleRhythmEvent, right: BattleRhythmEvent) {
  const normalize = (event: BattleRhythmEvent) => ({
    ...event,
    ownership:
      event.ownership.kind === "source"
        ? {
            kind: "source",
            sourceId: event.ownership.sourceId,
            sourceLocation: event.ownership.sourceLocation,
          }
        : event.ownership,
  });
  return JSON.stringify(normalize(left)) === JSON.stringify(normalize(right));
}

export function buildImportRevision(
  input: Readonly<{
    sourceId: string;
    revisionId: string;
    priorRevisionId: string | null;
    importedAt: string;
    timeZone: string;
    proposal: ImportProposal;
    existing: readonly BattleRhythmEvent[];
  }>,
): Readonly<{
  revision: BattleRhythmRevision;
  events: readonly BattleRhythmEvent[];
  diff: ImportDiff;
}> {
  const owned = input.existing.filter(
    (event) =>
      event.ownership.kind === "source" &&
      event.ownership.sourceId === input.sourceId,
  );
  const byLocation = new Map(
    owned.map((event) => [
      event.ownership.kind === "source" ? event.ownership.sourceLocation : "",
      event,
    ]),
  );
  const proposedLocations = new Set(
    input.proposal.events.map((event) => event.sourceLocation),
  );
  const changes: BattleRhythmRevision["changes"][number][] = [];
  const events: BattleRhythmEvent[] = [];
  let added = 0;
  let changed = 0;
  let unchanged = 0;
  for (const proposed of input.proposal.events) {
    const prior = byLocation.get(proposed.sourceLocation);
    const after = eventFromProposal(
      input.sourceId,
      input.revisionId,
      input.timeZone,
      proposed,
      prior,
    );
    if (!prior) {
      changes.push({ kind: "added", after });
      events.push(after);
      added += 1;
    } else if (!sameContent(prior, after)) {
      changes.push({ kind: "changed", before: prior, after });
      events.push(after);
      changed += 1;
    } else {
      unchanged += 1;
    }
  }
  let removed = 0;
  for (const prior of owned) {
    if (
      Date.parse(prior.start) >=
        Date.parse(input.proposal.proposedCoverage.start) &&
      Date.parse(prior.start) <
        Date.parse(input.proposal.proposedCoverage.end) &&
      prior.ownership.kind === "source" &&
      !proposedLocations.has(prior.ownership.sourceLocation)
    ) {
      changes.push({ kind: "removed", before: prior });
      removed += 1;
    }
  }
  return Object.freeze({
    revision: parseBattleRhythmRevision({
      schemaVersion: 1,
      id: input.revisionId,
      sourceId: input.sourceId,
      priorRevisionId: input.priorRevisionId,
      importedAt: input.importedAt,
      changes,
    }),
    events: Object.freeze(events),
    diff: Object.freeze({
      added,
      changed,
      removed,
      unchanged,
      preserved: input.existing.length - owned.length,
    }),
  });
}

export function supportsImportedSourceType(
  value: BattleRhythmSourceType,
): value is ImportProposal["sourceType"] {
  return value === "fas" || value === "longcast" || value === "shortcast";
}
