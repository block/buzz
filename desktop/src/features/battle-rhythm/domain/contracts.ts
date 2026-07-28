export type BattleRhythmSourceType = "fas" | "longcast" | "shortcast" | "other";
export type BattleRhythmStatus = "draft" | "approved" | "cancelled";
export type EventOwnership =
  | { readonly kind: "manual" }
  | {
      readonly kind: "source";
      readonly sourceId: string;
      readonly revisionId: string;
      readonly sourceLocation: string;
    };
export type BattleRhythmSource = Readonly<{
  schemaVersion: 1;
  id: string;
  type: BattleRhythmSourceType;
  displayName: string;
  coverageStart: string;
  coverageEnd: string;
  documentName: string;
  documentHash: string;
  revisionId: string;
  priorRevisionId: string | null;
  importedAt: string;
  status: BattleRhythmStatus;
  sourceReference: string;
}>;
export type BattleRhythmEvent = Readonly<{
  schemaVersion: 1;
  id: string;
  ownership: EventOwnership;
  title: string;
  description: string | null;
  type: string;
  start: string;
  end: string;
  allDay: boolean;
  timeZone: string;
  status: BattleRhythmStatus;
  location: string | null;
  responsibleOwner: string | null;
  participants: readonly string[];
  remarks: string | null;
  linkedPlanId: string | null;
  linkedTaskId: string | null;
  linkedMissionRequirementId: string | null;
  parentActivityId: string | null;
}>;
export type BattleRhythmChange = Readonly<
  | { kind: "added"; after: BattleRhythmEvent }
  | { kind: "changed"; before: BattleRhythmEvent; after: BattleRhythmEvent }
  | { kind: "removed"; before: BattleRhythmEvent }
>;
export type BattleRhythmRevision = Readonly<{
  schemaVersion: 1;
  id: string;
  sourceId: string;
  priorRevisionId: string | null;
  importedAt: string;
  changes: readonly BattleRhythmChange[];
}>;
export type BattleRhythmRevisionChunk = Readonly<{
  schemaVersion: 1;
  revisionId: string;
  sourceId: string;
  chunkIndex: number;
  chunkCount: number;
  manifestHash: string;
  changes: readonly BattleRhythmChange[];
}>;

const ISO =
  /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$/;
const statuses = new Set(["draft", "approved", "cancelled"]);
const sourceTypes = new Set(["fas", "longcast", "shortcast", "other"]);
const eventKeys = [
  "schemaVersion",
  "id",
  "ownership",
  "title",
  "description",
  "type",
  "start",
  "end",
  "allDay",
  "timeZone",
  "status",
  "location",
  "responsibleOwner",
  "participants",
  "remarks",
  "linkedPlanId",
  "linkedTaskId",
  "linkedMissionRequirementId",
  "parentActivityId",
];
function fail(message: string): never {
  throw new Error(`Invalid Battle Rhythm contract: ${message}`);
}
function record(
  value: unknown,
  keys: readonly string[],
): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value))
    fail("object required");
  const o = value as Record<string, unknown>;
  if (
    Object.keys(o).length !== keys.length ||
    Object.keys(o).some((key) => !keys.includes(key))
  )
    fail("unknown or missing field");
  return o;
}
function string(value: unknown, name: string, max = 4096): string {
  if (typeof value !== "string" || value.length === 0 || value.length > max)
    fail(`${name} must be bounded nonempty text`);
  return value;
}
function nullableString(value: unknown, name: string): string | null {
  return value === null ? null : string(value, name);
}
function timestamp(value: unknown, name: string): string {
  const raw = string(value, name, 64);
  if (!ISO.test(raw) || Number.isNaN(Date.parse(raw)))
    fail(`${name} must be ISO-8601`);
  return raw;
}
function status(value: unknown): BattleRhythmStatus {
  if (typeof value !== "string" || !statuses.has(value)) fail("invalid status");
  return value as BattleRhythmStatus;
}
function freeze<T extends object>(value: T): Readonly<T> {
  return Object.freeze(value);
}
export function parseBattleRhythmSource(value: unknown): BattleRhythmSource {
  const o = record(value, [
    "schemaVersion",
    "id",
    "type",
    "displayName",
    "coverageStart",
    "coverageEnd",
    "documentName",
    "documentHash",
    "revisionId",
    "priorRevisionId",
    "importedAt",
    "status",
    "sourceReference",
  ]);
  const start = timestamp(o.coverageStart, "coverageStart"),
    end = timestamp(o.coverageEnd, "coverageEnd");
  if (Date.parse(start) >= Date.parse(end)) fail("coverage must be ordered");
  if (typeof o.type !== "string" || !sourceTypes.has(o.type))
    fail("invalid source type");
  const hash = string(o.documentHash, "documentHash", 128);
  if (!/^[a-f0-9]{64}$/i.test(hash)) fail("documentHash must be sha256");
  return freeze({
    schemaVersion: one(o.schemaVersion),
    id: string(o.id, "id", 256),
    type: o.type as BattleRhythmSourceType,
    displayName: string(o.displayName, "displayName"),
    coverageStart: start,
    coverageEnd: end,
    documentName: string(o.documentName, "documentName"),
    documentHash: hash,
    revisionId: string(o.revisionId, "revisionId", 256),
    priorRevisionId: nullableString(o.priorRevisionId, "priorRevisionId"),
    importedAt: timestamp(o.importedAt, "importedAt"),
    status: status(o.status),
    sourceReference: string(o.sourceReference, "sourceReference"),
  });
}
function one(value: unknown): 1 {
  if (value !== 1) fail("schemaVersion must be 1");
  return 1;
}
function ownership(value: unknown): EventOwnership {
  if (!value || typeof value !== "object" || Array.isArray(value))
    fail("ownership object required");
  const o = value as Record<string, unknown>;
  if (o.kind === "manual") {
    if (Object.keys(o).length !== 1) fail("manual ownership has no source");
    return freeze({ kind: "manual" });
  }
  if (o.kind === "source" && Object.keys(o).length === 4)
    return freeze({
      kind: "source",
      sourceId: string(o.sourceId, "sourceId", 256),
      revisionId: string(o.revisionId, "revisionId", 256),
      sourceLocation: string(o.sourceLocation, "sourceLocation"),
    });
  fail("invalid ownership");
}
export function parseBattleRhythmEvent(value: unknown): BattleRhythmEvent {
  const o = record(value, eventKeys);
  const start = timestamp(o.start, "start"),
    end = timestamp(o.end, "end");
  if (Date.parse(start) >= Date.parse(end)) fail("start must precede end");
  if (typeof o.allDay !== "boolean") fail("allDay must be boolean");
  if (!Array.isArray(o.participants) || o.participants.length > 256)
    fail("participants invalid");
  return freeze({
    schemaVersion: one(o.schemaVersion),
    id: string(o.id, "id", 256),
    ownership: ownership(o.ownership),
    title: string(o.title, "title", 512),
    description: nullableString(o.description, "description"),
    type: string(o.type, "type", 128),
    start,
    end,
    allDay: o.allDay,
    timeZone: string(o.timeZone, "timeZone", 128),
    status: status(o.status),
    location: nullableString(o.location, "location"),
    responsibleOwner: nullableString(o.responsibleOwner, "responsibleOwner"),
    participants: freeze(
      o.participants.map((p) => string(p, "participant", 256)),
    ),
    remarks: nullableString(o.remarks, "remarks"),
    linkedPlanId: nullableString(o.linkedPlanId, "linkedPlanId"),
    linkedTaskId: nullableString(o.linkedTaskId, "linkedTaskId"),
    linkedMissionRequirementId: nullableString(
      o.linkedMissionRequirementId,
      "linkedMissionRequirementId",
    ),
    parentActivityId: nullableString(o.parentActivityId, "parentActivityId"),
  });
}
function change(value: unknown): BattleRhythmChange {
  if (!value || typeof value !== "object" || Array.isArray(value))
    fail("change object required");
  const o = value as Record<string, unknown>;
  if (o.kind === "added" && Object.keys(o).length === 2)
    return freeze({ kind: "added", after: parseBattleRhythmEvent(o.after) });
  if (o.kind === "removed" && Object.keys(o).length === 2)
    return freeze({
      kind: "removed",
      before: parseBattleRhythmEvent(o.before),
    });
  if (o.kind === "changed" && Object.keys(o).length === 3)
    return freeze({
      kind: "changed",
      before: parseBattleRhythmEvent(o.before),
      after: parseBattleRhythmEvent(o.after),
    });
  fail("invalid change");
}
export function parseBattleRhythmRevision(
  value: unknown,
): BattleRhythmRevision {
  const o = record(value, [
    "schemaVersion",
    "id",
    "sourceId",
    "priorRevisionId",
    "importedAt",
    "changes",
  ]);
  if (!Array.isArray(o.changes) || o.changes.length > 2000)
    fail("changes must contain at most 2000 entries");
  return freeze({
    schemaVersion: one(o.schemaVersion),
    id: string(o.id, "id", 256),
    sourceId: string(o.sourceId, "sourceId", 256),
    priorRevisionId: nullableString(o.priorRevisionId, "priorRevisionId"),
    importedAt: timestamp(o.importedAt, "importedAt"),
    changes: freeze(o.changes.map(change)),
  });
}
export function parseBattleRhythmRevisionChunk(
  value: unknown,
): BattleRhythmRevisionChunk {
  const o = record(value, [
    "schemaVersion",
    "revisionId",
    "sourceId",
    "chunkIndex",
    "chunkCount",
    "manifestHash",
    "changes",
  ]);
  if (
    !Number.isInteger(o.chunkIndex) ||
    !Number.isInteger(o.chunkCount) ||
    typeof o.chunkIndex !== "number" ||
    typeof o.chunkCount !== "number" ||
    o.chunkIndex < 0 ||
    o.chunkIndex >= o.chunkCount
  )
    fail("invalid chunk sequence");
  if (!Array.isArray(o.changes)) fail("chunk changes required");
  const manifestHash = string(o.manifestHash, "manifestHash", 128);
  if (!/^[a-f0-9]{64}$/i.test(manifestHash))
    fail("manifestHash must be sha256");
  return freeze({
    schemaVersion: one(o.schemaVersion),
    revisionId: string(o.revisionId, "revisionId", 256),
    sourceId: string(o.sourceId, "sourceId", 256),
    chunkIndex: o.chunkIndex,
    chunkCount: o.chunkCount,
    manifestHash,
    changes: freeze(o.changes.map(change)),
  });
}
