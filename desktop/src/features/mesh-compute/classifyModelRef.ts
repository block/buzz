/**
 * Classification of a free-text model ref entered into the serve card.
 * Mirrors mesh's own resolution categories for input validation.
 */
export type ModelRefKind =
  | { kind: "catalog"; name: string }
  | { kind: "huggingface"; ref: string }
  | { kind: "local-path"; path: string }
  | { kind: "unknown" };

/**
 * Classify a model-ref string the way mesh-llm's runtime does:
 *  - `hf://…` → HuggingFace ref
 *  - starts with `/` or `./` or `~`, OR ends with `.gguf` → local file
 *  - otherwise non-empty → catalog name
 *  - empty/whitespace → unknown
 *
 * Source: mesh runtime/mod.rs:3390 ("local file, catalog name, or HuggingFace URL").
 *
 * This is validation-only — canonical resolution happens server-side via
 * `mesh_start_node`.
 */
export function classifyModelRef(raw: string): ModelRefKind {
  const trimmed = raw.trim();
  if (trimmed.length === 0) {
    return { kind: "unknown" };
  }
  if (trimmed.startsWith("hf://")) {
    return { kind: "huggingface", ref: trimmed };
  }
  // Local path heuristics. Conservative: only mark as path when there are
  // unambiguous signals (leading separator, home shortcut, .gguf extension).
  const looksLikePath =
    trimmed.startsWith("/") ||
    trimmed.startsWith("./") ||
    trimmed.startsWith("../") ||
    trimmed.startsWith("~") ||
    trimmed.toLowerCase().endsWith(".gguf");
  if (looksLikePath) {
    return { kind: "local-path", path: trimmed };
  }
  return { kind: "catalog", name: trimmed };
}
