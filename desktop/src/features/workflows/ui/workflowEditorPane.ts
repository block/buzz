export type WorkflowEditorPane =
  | { type: "trigger" }
  | { type: "step"; stepId: string }
  | null;

const STEP_PANE_PREFIX = "step:";
const STEP_ID_PATTERN = /^[A-Za-z0-9_]{1,64}$/;

export function parseWorkflowEditorPane(value: unknown): WorkflowEditorPane {
  if (value === "trigger") return { type: "trigger" };
  if (typeof value !== "string" || !value.startsWith(STEP_PANE_PREFIX)) {
    return null;
  }

  const encodedStepId = value.slice(STEP_PANE_PREFIX.length);
  if (!encodedStepId || encodedStepId.length > 192) return null;
  try {
    const stepId = decodeURIComponent(encodedStepId);
    return STEP_ID_PATTERN.test(stepId) ? { type: "step", stepId } : null;
  } catch {
    return null;
  }
}

export function serializeWorkflowEditorPane(
  pane: WorkflowEditorPane,
): string | undefined {
  if (pane === null) return undefined;
  if (pane.type === "trigger") return "trigger";
  if (!STEP_ID_PATTERN.test(pane.stepId)) return undefined;
  return `${STEP_PANE_PREFIX}${encodeURIComponent(pane.stepId)}`;
}
