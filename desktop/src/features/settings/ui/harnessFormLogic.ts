/**
 * Pure logic helpers for the custom-harness form.
 *
 * Extracted so they can be tested independently from the React layer.
 */

/**
 * Derive a valid harness ID from a human-readable display name.
 *
 * Rules match the backend `is_valid_harness_id` predicate:
 *   [a-z0-9_][a-z0-9_-]*
 *
 * The transform:
 *   1. Lowercase the label.
 *   2. Replace any character outside [a-z0-9_-] with a hyphen.
 *   3. Strip a leading character that isn't [a-z0-9_] (covers e.g. "-foo").
 *   4. Collapse runs of hyphens.
 *   5. Strip trailing hyphens.
 */
export function idFromLabel(label: string): string {
  return label
    .toLowerCase()
    .replace(/[^a-z0-9_-]/g, "-")
    .replace(/^[^a-z0-9_]/, "")
    .replace(/-{2,}/g, "-")
    .replace(/-+$/, "");
}

/**
 * Build the env-variable record that goes into the save payload.
 *
 * Pairs with empty keys are silently skipped — they represent blank
 * rows the user hasn't filled in yet.
 */
export function buildEnvRecord(
  pairs: ReadonlyArray<{ key: string; value: string }>,
): Record<string, string> {
  const out: Record<string, string> = {};
  for (const { key, value } of pairs) {
    const k = key.trim();
    if (k) out[k] = value;
  }
  return out;
}

/**
 * Convert a `Record<string, string>` env map (as stored on a catalog entry or
 * harness definition) into the array of `{ key, value }` pairs that
 * `CustomHarnessForm` and `EnvEditor` work with.
 *
 * Used when opening the edit form for an existing custom harness so that
 * existing env vars are pre-filled instead of silently reset to empty.
 */
export function envPairsFromRecord(
  record: Record<string, string> | undefined,
): Array<{ key: string; value: string }> {
  if (!record) return [];
  return Object.entries(record).map(([key, value]) => ({ key, value }));
}

/**
 * Remove empty/whitespace-only argument rows from the args list before
 * sending to the backend.  Preserves legitimate blank-string args only
 * when they contain non-whitespace content so quoted/complex args are
 * never silently dropped.
 */
export function filterArgs(args: ReadonlyArray<string>): string[] {
  return args.filter((a) => a.trim() !== "");
}

/** Form state for the custom-harness create/edit form. */
export interface CustomFormValues {
  id: string;
  label: string;
  command: string;
  /** Each element is one argument; no space-splitting round-trip. */
  args: string[];
  /** KEY=VALUE pairs for env injection at spawn time. */
  env: Array<{ key: string; value: string }>;
  installInstructionsUrl: string;
  installHint: string;
}

/** The subset of a catalog entry that seeds the edit form. */
export interface EditableCatalogEntry {
  id: string;
  label: string;
  command: string | null;
  defaultArgs: string[] | undefined;
  definitionEnv?: Record<string, string>;
  installInstructionsUrl: string;
  installHint: string;
}

/** Definition payload sent to `save_custom_harness`. */
export interface HarnessDefinitionPayload {
  id: string;
  label: string;
  command: string;
  args: string[];
  env: Record<string, string>;
  installInstructionsUrl: string;
  installHint: string;
}

/**
 * Seed form values from an existing catalog entry (edit flow).
 *
 * This is the single source of truth for edit initialization: every
 * user-editable definition field must appear here, or an open-edit-then-save
 * silently erases it. `formValuesRoundTrip` in the tests proves the
 * entry → form → save-payload loop is lossless.
 */
export function formValuesFromCatalogEntry(
  entry: EditableCatalogEntry,
): CustomFormValues {
  return {
    id: entry.id,
    label: entry.label,
    command: entry.command ?? "",
    args: entry.defaultArgs ?? [],
    env: envPairsFromRecord(entry.definitionEnv),
    installInstructionsUrl: entry.installInstructionsUrl,
    installHint: entry.installHint,
  };
}

/** Build the `save_custom_harness` definition payload from form values. */
export function definitionFromFormValues(
  form: CustomFormValues,
): HarnessDefinitionPayload {
  return {
    id: form.id.trim(),
    label: form.label.trim(),
    command: form.command.trim(),
    args: filterArgs(form.args),
    env: buildEnvRecord(form.env),
    installInstructionsUrl: form.installInstructionsUrl.trim(),
    installHint: form.installHint.trim(),
  };
}

/**
 * Client-side mirror of the backend comma-in-args rejection.
 *
 * Args travel to the harness through the comma-delimited
 * `BUZZ_ACP_AGENT_ARGS` env transport (clap `value_delimiter = ','` on the
 * buzz-acp side), so a literal comma inside one argument would silently split
 * into two at spawn time. The backend validator rejects it; this mirror gives
 * the user an inline error naming the offending argument before the round-trip.
 *
 * Returns the error message, or `null` when all args are comma-free.
 */
export function commaArgError(args: ReadonlyArray<string>): string | null {
  const offender = args.find((a) => a.includes(","));
  if (offender === undefined) return null;
  return `Argument "${offender}" contains a comma — arguments are passed via a comma-delimited transport and would be split at spawn time. Use separate argument entries instead.`;
}
