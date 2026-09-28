/**
 * Parse agent ```bestie-scratch``` fence actions.
 *
 * ```bestie-scratch
 * {"op":"add","title":"Idea","body":"park this for later"}
 * ```
 */

export type BestieScratchFenceAction =
  | { note: { body?: string; title?: string }; op: "add" }
  | { body?: string; id: string; op: "update"; title?: string }
  | { id: string; op: "remove" };

const FENCE_RE = /```bestie-scratch\s*\r?\n([\s\S]*?)```/gi;

function parseOne(raw: unknown): BestieScratchFenceAction | null {
  if (typeof raw !== "object" || raw === null) return null;
  const record = raw as Record<string, unknown>;
  if (record.op === "add") {
    const title = typeof record.title === "string" ? record.title : "";
    const body = typeof record.body === "string" ? record.body : "";
    if (!title.trim() && !body.trim()) return null;
    return { note: { body, title }, op: "add" };
  }
  if (record.op === "update" && typeof record.id === "string" && record.id) {
    return {
      body: typeof record.body === "string" ? record.body : undefined,
      id: record.id,
      op: "update",
      title: typeof record.title === "string" ? record.title : undefined,
    };
  }
  if (record.op === "remove" && typeof record.id === "string" && record.id) {
    return { id: record.id, op: "remove" };
  }
  return null;
}

export function parseBestieScratchActionsFromMessage(
  content: string,
): BestieScratchFenceAction[] {
  const actions: BestieScratchFenceAction[] = [];
  for (const match of content.matchAll(FENCE_RE)) {
    const block = (match[1] ?? "").trim();
    if (!block) continue;
    try {
      const parsed: unknown = JSON.parse(block);
      const list = Array.isArray(parsed) ? parsed : [parsed];
      for (const entry of list) {
        const action = parseOne(entry);
        if (action) actions.push(action);
      }
    } catch {
      // Ignore malformed fences.
    }
  }
  return actions;
}
