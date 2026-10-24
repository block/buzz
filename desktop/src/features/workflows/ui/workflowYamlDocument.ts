import { isMap, parseDocument } from "yaml";

import { DEFAULT_FORM_STATE, formStateToYaml } from "./workflowFormTypes";

/**
 * Header-level view of a workflow definition.
 *
 * The dialog header (name, name editing, enabled toggle) must stay usable while
 * the body of the definition is still incomplete — a freshly added step has no
 * message text yet, so full form validation fails even though `name` is present
 * and editable. These fields therefore come from the YAML document itself
 * rather than from {@link yamlToFormState}.
 */
export type WorkflowDocumentFields = {
  /** Whether top-level keys can be written back into the document. */
  editable: boolean;
  /** The explicit `enabled` value, or `null` when the key is absent. */
  enabled: boolean | null;
  /** The top-level `name` scalar, or `null` when absent or not a string. */
  name: string | null;
};

const EMPTY_DOCUMENT_FIELDS: WorkflowDocumentFields = {
  editable: true,
  enabled: null,
  name: null,
};

const UNEDITABLE_DOCUMENT_FIELDS: WorkflowDocumentFields = {
  editable: false,
  enabled: null,
  name: null,
};

/**
 * Reads the header-level fields of a workflow definition without requiring the
 * whole definition to be valid.
 */
export function readWorkflowDocumentFields(
  yaml: string,
): WorkflowDocumentFields {
  if (!yaml.trim()) return EMPTY_DOCUMENT_FIELDS;

  const document = parseDocument(yaml);
  if (document.errors.length > 0 || !isMap(document.contents)) {
    return UNEDITABLE_DOCUMENT_FIELDS;
  }

  const name = document.get("name");
  const enabled = document.get("enabled");
  return {
    editable: true,
    enabled: typeof enabled === "boolean" ? enabled : null,
    name: typeof name === "string" ? name : null,
  };
}

/** What the dialog header renders for a definition, in a single parse. */
export type WorkflowHeaderState = {
  /** Whether the name can be edited and the enabled toggle can be written. */
  canEdit: boolean;
  enabled: boolean;
  name: string;
};

/**
 * Derives the header presentation from the working definition, falling back to
 * the saved workflow only for what the document itself does not supply.
 */
export function readWorkflowHeaderState(
  yaml: string,
  fallback: { enabled: boolean; name: string | undefined },
): WorkflowHeaderState {
  const fields = readWorkflowDocumentFields(yaml);
  return {
    canEdit: fields.editable,
    enabled: fields.editable ? fields.enabled !== false : fallback.enabled,
    name: fields.name?.trim() || (fallback.name?.trim() ?? ""),
  };
}

/** Writes `name` into the definition, preserving everything else verbatim. */
export function yamlWithWorkflowName(
  yaml: string,
  name: string,
): string | null {
  if (!yaml.trim()) {
    return formStateToYaml({ ...DEFAULT_FORM_STATE, name });
  }

  const document = parseDocument(yaml);
  if (document.errors.length > 0 || !isMap(document.contents)) return null;
  document.set("name", name);
  return document.toString();
}

/** Writes `enabled` into the definition, preserving everything else verbatim. */
export function yamlWithWorkflowEnabled(
  yaml: string,
  enabled: boolean,
): string | null {
  if (!yaml.trim()) {
    return formStateToYaml({ ...DEFAULT_FORM_STATE, enabled });
  }

  const document = parseDocument(yaml);
  if (document.errors.length > 0 || !isMap(document.contents)) return null;
  if (enabled) {
    document.delete("enabled");
  } else {
    document.set("enabled", false);
  }
  return document.toString();
}
