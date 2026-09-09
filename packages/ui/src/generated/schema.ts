/** The finite set of status meanings supported in generated views. */
export type GeneratedTone = "info" | "success" | "warning" | "danger";
/** Only these explicit, non-executable blocks may be produced by an agent. */
export type GeneratedBlock =
  | { id: string; type: "text"; text: string }
  | { id: string; type: "notice"; tone: GeneratedTone; text: string }
  | {
      id: string;
      type: "metric";
      label: string;
      value: string;
      detail?: string;
    }
  | {
      id: string;
      type: "tasks";
      items: {
        id: string;
        label: string;
        status: "pending" | "running" | "complete" | "failed";
      }[];
    }
  | { id: string; type: "table"; columns: string[]; rows: string[][] }
  | { id: string; type: "actions"; items: { id: string; label: string }[] };
/** A complete versioned snapshot. Streaming transport remains the host's responsibility. */
export interface GeneratedView {
  version: 1;
  title: string;
  blocks: GeneratedBlock[];
}
/** Validation either returns a safe typed snapshot or an actionable failure. */
export type GeneratedViewResult =
  | { ok: true; value: GeneratedView }
  | { ok: false; error: string };
const MAX_JSON = 64000;
const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);
const text = (value: unknown, max = 4000): value is string =>
  typeof value === "string" && value.length > 0 && value.length <= max;
const id = (value: unknown): value is string =>
  text(value, 64) && /^[a-zA-Z0-9_-]+$/.test(value);
const keys = (value: Record<string, unknown>, allowed: string[]) =>
  Object.keys(value).every((key) => allowed.includes(key));
const list = (value: unknown, max: number): value is unknown[] =>
  Array.isArray(value) && value.length > 0 && value.length <= max;
const unique = (items: unknown[], key: (item: unknown) => unknown) =>
  new Set(items.map(key)).size === items.length;
const itemId = (item: unknown) => (isRecord(item) ? item.id : undefined);
/** Parse and bound an untrusted JSON snapshot. No HTML, URLs, styles, or executable props are accepted. */
export function parseGeneratedView(input: unknown): GeneratedViewResult {
  let value: unknown;
  try {
    const serialized =
      typeof input === "string" ? input : JSON.stringify(input);
    if (!serialized || serialized.length > MAX_JSON)
      return {
        ok: false,
        error: "Response exceeds the 64,000-character limit.",
      };
    value = JSON.parse(serialized);
  } catch {
    return { ok: false, error: "Response is not valid JSON." };
  }
  if (
    !isRecord(value) ||
    value.version !== 1 ||
    !text(value.title, 200) ||
    !keys(value, ["version", "title", "blocks"]) ||
    !list(value.blocks, 20) ||
    !unique(value.blocks, itemId)
  )
    return {
      ok: false,
      error:
        "Expected a version 1 response with a title and 1–20 uniquely named blocks.",
    };
  const actionIds = new Set<string>();
  for (const block of value.blocks) {
    if (!isRecord(block) || !id(block.id) || typeof block.type !== "string")
      return {
        ok: false,
        error: "Every block needs a valid id and supported type.",
      };
    let valid = false;
    switch (block.type) {
      case "text":
        valid = keys(block, ["id", "type", "text"]) && text(block.text);
        break;
      case "notice":
        valid =
          keys(block, ["id", "type", "tone", "text"]) &&
          ["info", "success", "warning", "danger"].includes(
            String(block.tone),
          ) &&
          text(block.text);
        break;
      case "metric":
        valid =
          keys(block, ["id", "type", "label", "value", "detail"]) &&
          text(block.label, 200) &&
          text(block.value, 100) &&
          (block.detail === undefined || text(block.detail, 500));
        break;
      case "tasks":
        valid =
          keys(block, ["id", "type", "items"]) &&
          list(block.items, 30) &&
          unique(block.items, itemId) &&
          block.items.every(
            (item) =>
              isRecord(item) &&
              keys(item, ["id", "label", "status"]) &&
              id(item.id) &&
              text(item.label, 500) &&
              ["pending", "running", "complete", "failed"].includes(
                String(item.status),
              ),
          );
        break;
      case "table":
        valid =
          keys(block, ["id", "type", "columns", "rows"]) &&
          list(block.columns, 8) &&
          block.columns.every((column) => text(column, 100)) &&
          unique(block.columns, (item) => item) &&
          list(block.rows, 50) &&
          block.rows.every(
            (row) =>
              Array.isArray(row) &&
              row.length === (block.columns as unknown[]).length &&
              row.every(
                (cell) => typeof cell === "string" && cell.length <= 500,
              ),
          );
        break;
      case "actions":
        valid =
          keys(block, ["id", "type", "items"]) &&
          list(block.items, 6) &&
          block.items.every((item) => {
            if (
              !isRecord(item) ||
              !keys(item, ["id", "label"]) ||
              !id(item.id) ||
              !text(item.label, 100) ||
              actionIds.has(item.id)
            )
              return false;
            actionIds.add(item.id);
            return true;
          });
        break;
    }
    if (!valid)
      return {
        ok: false,
        error: `Block ${block.id} does not match the supported ${block.type} schema.`,
      };
  }
  return { ok: true, value: value as unknown as GeneratedView };
}
