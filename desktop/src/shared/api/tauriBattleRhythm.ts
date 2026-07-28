import { invokeTauri } from "@/shared/api/tauri";

export type ExtractedPlanningBlock =
  | Readonly<{
      kind: "table_row";
      location: string;
      cells: readonly string[];
    }>
  | Readonly<{
      kind: "spreadsheet_cell";
      location: string;
      sheet: string;
      coordinate: string;
      value: string;
    }>
  | Readonly<{
      kind: "spreadsheet_merge";
      location: string;
      sheet: string;
      range: string;
    }>
  | Readonly<{
      kind: "pdf_page";
      location: string;
      page: number;
      text: string;
      confidence: number | null;
    }>;

export type ExtractedPlanningDocument = Readonly<{
  filename: string;
  extension: "docx" | "xlsx" | "pdf";
  sha256: string;
  sizeBytes: number;
  blocks: readonly ExtractedPlanningBlock[];
  pages: readonly number[];
  sheets: readonly Readonly<{
    name: string;
    maximumRow: number;
    maximumColumn: number;
  }>[];
  truncated: boolean;
}>;

function record(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value))
    throw new Error("Battle Rhythm returned an invalid extracted document.");
  return value as Record<string, unknown>;
}

function exact(value: Record<string, unknown>, keys: readonly string[]) {
  const actual = Object.keys(value);
  if (
    actual.length !== keys.length ||
    actual.some((key) => !keys.includes(key))
  )
    throw new Error("Battle Rhythm returned an invalid extracted document.");
}

function text(value: unknown, maximum = 1024 * 1024): string {
  if (typeof value !== "string" || value.length === 0 || value.length > maximum)
    throw new Error("Battle Rhythm returned an invalid extracted document.");
  return value;
}

function integer(value: unknown, maximum: number): number {
  if (
    !Number.isSafeInteger(value) ||
    (value as number) < 0 ||
    (value as number) > maximum
  )
    throw new Error("Battle Rhythm returned an invalid extracted document.");
  return value as number;
}

function parseBlock(value: unknown): ExtractedPlanningBlock {
  const block = record(value);
  if (block.kind === "table_row") {
    exact(block, ["kind", "location", "cells"]);
    if (
      !Array.isArray(block.cells) ||
      block.cells.length > 256 ||
      !block.cells.every((cell) => typeof cell === "string")
    )
      throw new Error("Battle Rhythm returned an invalid extracted document.");
    return Object.freeze({
      kind: block.kind,
      location: text(block.location),
      cells: Object.freeze(block.cells.map((cell) => text(cell, 65_536))),
    });
  }
  if (block.kind === "spreadsheet_cell" || block.kind === "spreadsheet_merge") {
    const last = block.kind === "spreadsheet_cell" ? "value" : "range";
    exact(block, [
      "kind",
      "location",
      "sheet",
      block.kind === "spreadsheet_cell" ? "coordinate" : "range",
      ...(block.kind === "spreadsheet_cell" ? ["value"] : []),
    ]);
    const common = {
      kind: block.kind,
      location: text(block.location),
      sheet: text(block.sheet),
    };
    return block.kind === "spreadsheet_cell"
      ? Object.freeze({
          ...common,
          kind: block.kind,
          coordinate: text(block.coordinate),
          value: text(block[last], 65_536),
        })
      : Object.freeze({
          ...common,
          kind: block.kind,
          range: text(block[last]),
        });
  }
  if (block.kind === "pdf_page") {
    exact(block, ["kind", "location", "page", "text", "confidence"]);
    if (
      block.confidence !== null &&
      (typeof block.confidence !== "number" ||
        block.confidence < 0 ||
        block.confidence > 1)
    )
      throw new Error("Battle Rhythm returned an invalid extracted document.");
    return Object.freeze({
      kind: block.kind,
      location: text(block.location),
      page: integer(block.page, 2_000),
      text: text(block.text, 1024 * 1024),
      confidence: block.confidence as number | null,
    });
  }
  throw new Error("Battle Rhythm returned an invalid extracted document.");
}

function parseDocument(value: unknown): ExtractedPlanningDocument {
  const document = record(value);
  exact(document, [
    "filename",
    "extension",
    "sha256",
    "sizeBytes",
    "blocks",
    "pages",
    "sheets",
    "truncated",
  ]);
  if (
    !["docx", "xlsx", "pdf"].includes(document.extension as string) ||
    typeof document.sha256 !== "string" ||
    !/^[a-f0-9]{64}$/i.test(document.sha256) ||
    !Array.isArray(document.blocks) ||
    document.blocks.length > 20_000 ||
    !Array.isArray(document.pages) ||
    !Array.isArray(document.sheets) ||
    document.sheets.length > 256 ||
    typeof document.truncated !== "boolean"
  )
    throw new Error("Battle Rhythm returned an invalid extracted document.");
  const sheets = document.sheets.map((value) => {
    const sheet = record(value);
    exact(sheet, ["name", "maximumRow", "maximumColumn"]);
    return Object.freeze({
      name: text(sheet.name),
      maximumRow: integer(sheet.maximumRow, 1_048_576),
      maximumColumn: integer(sheet.maximumColumn, 16_384),
    });
  });
  return Object.freeze({
    filename: text(document.filename),
    extension: document.extension as ExtractedPlanningDocument["extension"],
    sha256: document.sha256,
    sizeBytes: integer(document.sizeBytes, 50 * 1024 * 1024),
    blocks: Object.freeze(document.blocks.map(parseBlock)),
    pages: Object.freeze(document.pages.map((page) => integer(page, 2_000))),
    sheets: Object.freeze(sheets),
    truncated: document.truncated,
  });
}

export async function pickBattleRhythmDocument(): Promise<ExtractedPlanningDocument | null> {
  const value = await invokeTauri<unknown>("pick_battle_rhythm_document", {});
  return value === null ? null : parseDocument(value);
}

export async function interpretBattleRhythmDocument(
  document: ExtractedPlanningDocument,
  sourceType: "fas" | "longcast" | "shortcast",
  proposedCoverage: Readonly<{ start: string; end: string }>,
): Promise<unknown | null> {
  return invokeTauri<unknown | null>("interpret_battle_rhythm_document", {
    request: { document, sourceType, proposedCoverage },
  });
}
