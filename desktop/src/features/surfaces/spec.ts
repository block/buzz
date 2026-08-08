import { z } from "zod";

// SurfaceSpec v1 — tolerant client-side parser.
//
// The relay validates strictly at ingest (buzz-core/src/surface.rs); this
// parser is deliberately tolerant for historical/foreign events: unknown
// tones coerce to "default", numeric cells are kept as numbers and rendered
// as text, invalid nodes drop individually while their siblings survive.
// Anything worse falls back to `fallbackText`, then to escaped raw content —
// never through the markdown pipeline, never a blank or error row.
//
// Structural limits mirror buzz-core; a node that exceeds them is treated as
// invalid (dropped) rather than truncated, so both gates agree on what a
// valid node is.

export const SURFACE_MAX_NODES = 32;
const MAX_SCALAR = 512;
const MAX_TEXT = 4096;
const MAX_ITEMS = 32;
const MAX_COLUMNS = 12;
const MAX_ROWS = 100;

const TONES = ["default", "success", "warning", "danger", "info"] as const;
export type SurfaceTone = (typeof TONES)[number];

const toneSchema = z.enum(TONES).catch("default");

const finiteNumber = z.number().refine(Number.isFinite);

// Length limits count Unicode code points ([...s].length), matching the
// relay's chars().count() — JS .length counts UTF-16 code units and would
// reject relay-valid strings containing astral-plane characters (emoji).
const maxCodePoints = (max: number) => (s: string) => [...s].length <= max;

const scalarSchema = z.union([
  z.string().refine(maxCodePoints(MAX_SCALAR)),
  finiteNumber,
]);

const nonBlank = (max: number) =>
  z
    .string()
    .refine(maxCodePoints(max))
    .refine((s) => s.trim().length > 0);

const headingSchema = z.object({
  type: z.literal("heading"),
  text: nonBlank(MAX_SCALAR),
});

const textSchema = z.object({
  type: z.literal("text"),
  text: nonBlank(MAX_TEXT),
});

const badgeSchema = z.object({
  type: z.literal("badge"),
  text: nonBlank(MAX_SCALAR),
  tone: toneSchema.default("default"),
});

const keyValueSchema = z.object({
  type: z.literal("keyValue"),
  items: z
    .array(
      z.object({
        label: nonBlank(MAX_SCALAR),
        value: scalarSchema,
        tone: toneSchema.default("default"),
      }),
    )
    .min(1)
    .max(MAX_ITEMS),
});

const statGridSchema = z.object({
  type: z.literal("statGrid"),
  stats: z
    .array(
      z.object({
        label: nonBlank(MAX_SCALAR),
        value: scalarSchema,
        delta: scalarSchema.optional(),
        tone: toneSchema.default("default"),
      }),
    )
    .min(1)
    .max(MAX_ITEMS),
});

const tableSchema = z
  .object({
    type: z.literal("table"),
    columns: z.array(nonBlank(MAX_SCALAR)).min(1).max(MAX_COLUMNS),
    rows: z.array(z.array(scalarSchema)).max(MAX_ROWS),
  })
  .refine((t) => t.rows.every((row) => row.length === t.columns.length));

const progressSchema = z.object({
  type: z.literal("progress"),
  label: nonBlank(MAX_SCALAR).optional(),
  // Client clamps to 0–100 (§3.7); the relay only requires finite.
  value: finiteNumber.transform((v) => Math.min(100, Math.max(0, v))),
});

const nodeSchema = z.discriminatedUnion("type", [
  headingSchema,
  textSchema,
  badgeSchema,
  keyValueSchema,
  statGridSchema,
  tableSchema,
  progressSchema,
]);

export type SurfaceNode = z.infer<typeof nodeSchema>;

export interface SurfaceSpec {
  title?: string;
  fallbackText: string;
  nodes: SurfaceNode[];
}

// Envelope shape probed before node-level salvage. Extra fields are ignored
// (tolerant), version is checked by hand so unknown versions can still route
// to the fallbackText path.
const envelopeSchema = z.object({
  version: z.number(),
  fallbackText: z
    .string()
    .refine(maxCodePoints(1024))
    .refine((s) => s.trim().length > 0),
  title: z.string().refine(maxCodePoints(512)).optional(),
  nodes: z.array(z.unknown()),
});

// Salvage a usable fallbackText from an envelope that failed schema parse
// (unknown future shape, nodes not an array, etc.). The fallback matrix
// prefers plain fallbackText over raw JSON whenever one is present.
function salvageFallbackText(json: unknown): string | null {
  if (typeof json !== "object" || json === null) {
    return null;
  }
  const value = (json as { fallbackText?: unknown }).fallbackText;
  if (typeof value !== "string" || value.trim().length === 0) {
    return null;
  }
  return [...value].length <= 1024 ? value : null;
}

export type SurfaceParseResult =
  | { outcome: "card"; spec: SurfaceSpec }
  | { outcome: "fallback"; text: string }
  | { outcome: "raw" };

/**
 * Parse surface event content per the v1 fallback matrix.
 *
 * - valid v1 spec → `card` (invalid nodes dropped, survivors render)
 * - unknown version / zero valid nodes → `fallback` with `fallbackText`
 * - unparseable JSON / missing fallbackText → `raw` (caller shows escaped
 *   plain text — NEVER the markdown pipeline)
 */
export function parseSurfaceSpec(raw: string): SurfaceParseResult {
  let json: unknown;
  try {
    json = JSON.parse(raw);
  } catch {
    return { outcome: "raw" };
  }

  const envelope = envelopeSchema.safeParse(json);
  if (!envelope.success) {
    const salvaged = salvageFallbackText(json);
    return salvaged
      ? { outcome: "fallback", text: salvaged }
      : { outcome: "raw" };
  }

  if (envelope.data.version !== 1) {
    return { outcome: "fallback", text: envelope.data.fallbackText };
  }

  const nodes: SurfaceNode[] = [];
  for (const rawNode of envelope.data.nodes.slice(0, SURFACE_MAX_NODES)) {
    const parsed = nodeSchema.safeParse(rawNode);
    if (parsed.success) {
      nodes.push(parsed.data);
    }
  }

  if (nodes.length === 0) {
    return { outcome: "fallback", text: envelope.data.fallbackText };
  }

  const title = envelope.data.title?.trim();
  return {
    outcome: "card",
    spec: {
      title: title ? title : undefined,
      fallbackText: envelope.data.fallbackText,
      nodes,
    },
  };
}

/** Render a scalar cell value for display. */
export function formatScalar(value: string | number): string {
  return typeof value === "number" ? String(value) : value;
}

/** True when the value should render with tabular-nums (numeric column). */
export function isNumeric(value: string | number): boolean {
  return typeof value === "number";
}

/**
 * Plain-text preview for a surface event's content — for notification
 * bodies, home-feed previews, and any other single-line context. Returns
 * `fallbackText` when the envelope carries one, else a generic label.
 * Never returns raw spec JSON.
 */
export function surfacePreviewText(content: string): string {
  const parsed = parseSurfaceSpec(content);
  switch (parsed.outcome) {
    case "card":
      return parsed.spec.fallbackText;
    case "fallback":
      return parsed.text;
    case "raw":
      return "Surface card";
  }
}

/**
 * Plain-text preview for any message body, given its kind. Surfaces render
 * their `fallbackText`; every other kind is returned unchanged.
 *
 * Use this anywhere a message body is shown outside its own renderer — reply
 * banners, reminder previews, notifications — so spec JSON never leaks into a
 * human-facing string.
 */
export function messagePreviewText(body: string, kind?: number): string {
  return kind === 40110 ? surfacePreviewText(body) : body;
}
