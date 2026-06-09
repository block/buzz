/**
 * Classification of a free-text model ref entered into the serve card.
 * UI shows a hint inline ("Looks like a catalog name") for trust feedback.
 * Mirrors mesh's own resolve logic at `runtime/mod.rs:3390`.
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
 * This is presentational only — the canonical resolution still happens server-
 * side via `mesh_start_node`. UI uses this for the "Looks like a …" hint that
 * makes the free-text field feel honest instead of opaque.
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

/** Short label for the inline hint, e.g. "Looks like a catalog name". */
export function modelRefHintLabel(kind: ModelRefKind): string | null {
  switch (kind.kind) {
    case "catalog":
      return "Looks like a catalog name";
    case "huggingface":
      return "HuggingFace ref";
    case "local-path":
      return "Local file";
    case "unknown":
      return null;
  }
}
