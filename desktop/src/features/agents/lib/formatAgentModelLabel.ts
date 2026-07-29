const DATABRICKS_PREFIX = "databricks-";

const MODEL_WORD_LABELS: Readonly<Record<string, string>> = {
  bge: "BGE",
  claude: "Claude",
  en: "EN",
  glm: "GLM",
  gpt: "GPT",
  gte: "GTE",
  llama: "Llama",
  meta: "Meta",
  mlflow: "MLflow",
  openai: "OpenAI",
};

/**
 * Turns a Databricks gateway endpoint into its human-facing model name while
 * preserving the endpoint ID everywhere it is sent to the runtime.
 *
 * Examples:
 * - `databricks-claude-opus-4-7` → `Claude Opus 4.7`
 * - `databricks-gpt-5-5` → `GPT-5.5`
 */
export function formatModelDisplayName(model: string | null | undefined) {
  const trimmed = model?.trim();
  if (!trimmed) return "";

  if (!trimmed.toLowerCase().startsWith(DATABRICKS_PREFIX)) {
    return trimmed;
  }

  const parts = trimmed.slice(DATABRICKS_PREFIX.length).split("-");
  const labelParts: string[] = [];
  for (let index = 0; index < parts.length; index += 1) {
    const part = parts[index];
    const nextPart = parts[index + 1];

    if (/^\d+$/.test(part) && nextPart && /^\d+$/.test(nextPart)) {
      labelParts.push(`${part}.${nextPart}`);
      index += 1;
      continue;
    }

    if (/^\d+b$/i.test(part)) {
      labelParts.push(part.toUpperCase());
      continue;
    }

    labelParts.push(
      MODEL_WORD_LABELS[part.toLowerCase()] ??
        `${part.slice(0, 1).toUpperCase()}${part.slice(1)}`,
    );
  }

  const label = labelParts.join(" ");
  return label.replace(/^GPT (\d)/, "GPT-$1");
}

/**
 * Returns a human-readable model label for an agent or persona, falling back to
 * "Auto" when no model is set (empty or whitespace-only).
 */
export function formatAgentModelLabel(model: string | null | undefined) {
  return formatModelDisplayName(model) || "Auto";
}
