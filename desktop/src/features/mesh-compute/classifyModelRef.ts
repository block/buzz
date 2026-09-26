/**
 * Classification of a free-text model ref entered into the serve card.
 * Mirrors mesh's own resolution categories for input validation.
 */
export type ModelRefKind =
  | { kind: "catalog"; name: string }
  | { kind: "huggingface"; ref: string }
  | { kind: "unknown" };

/**
 * Classify a model-ref string the way mesh-llm's runtime does:
 *  - `hf://…` → HuggingFace ref
 *  - otherwise non-empty → catalog name
 *  - empty/whitespace → unknown
 *
 * Local filesystem paths are classified `unknown`, NOT `local-path`: the
 * vendored mesh-llm SDK's `parse_exact_model_ref` has no local-path variant,
 * so feeding a path into `mesh_start_node` always fails with "Expected an
 * exact model ref". Treating paths as unknown keeps the Start button
 * disabled with a clear UI-level signal instead of a runtime error at start.
 * Reclassify-and-accept if/when the SDK gains a local-path branch.
 * See https://github.com/block/buzz/issues/4049.
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
  // Local path heuristics — classify as unknown so the form refuses to
  // submit. See the function-level doc-comment above and issue #4049.
  const looksLikePath =
    trimmed.startsWith("/") ||
    trimmed.startsWith("./") ||
    trimmed.startsWith("../") ||
    trimmed.startsWith("~") ||
    trimmed.toLowerCase().endsWith(".gguf") ||
    trimmed.startsWith("file://");
  if (looksLikePath) {
    return { kind: "unknown" };
  }
  return { kind: "catalog", name: trimmed };
}
