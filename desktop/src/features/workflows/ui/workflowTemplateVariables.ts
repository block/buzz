import { i18n } from "@/i18n";
import type {
  ActionType,
  StepFormState,
  TriggerType,
} from "./workflowFormTypes";

export type WorkflowTemplateVariableGroup = "previous-steps" | "trigger";

export type WorkflowTemplateVariable = {
  description: string;
  group: WorkflowTemplateVariableGroup;
  groupLabel: string;
  value: string;
};

export type ActiveTemplateToken = {
  end: number;
  query: string;
  start: number;
};

/** Caption ids are stable names; only the catalog text changes with language. */
type TemplateVariableCaption =
  | "author-pubkey"
  | "channel-uuid"
  | "diff-text"
  | "http-response-body"
  | "http-response-status"
  | "message-event-id"
  | "message-text"
  | "message-was-sent"
  | "reaction-emoji"
  | "reaction-was-added"
  | "seconds-elapsed"
  | "sent-message-id"
  | "unix-timestamp"
  | "workflow-channel-uuid";

/**
 * Template variable captions resolve at call time so they follow the live
 * language. The `value` tokens themselves are substituted verbatim into the
 * workflow YAML, so they stay exactly as written.
 */
export function templateVariableCaption(caption: TemplateVariableCaption) {
  switch (caption) {
    case "author-pubkey":
      return i18n.t("workflows.template.author-pubkey");
    case "channel-uuid":
      return i18n.t("workflows.template.channel-uuid");
    case "diff-text":
      return i18n.t("workflows.template.diff-text");
    case "http-response-body":
      return i18n.t("workflows.template.http-response-body");
    case "http-response-status":
      return i18n.t("workflows.template.http-response-status");
    case "message-event-id":
      return i18n.t("workflows.template.message-event-id");
    case "message-text":
      return i18n.t("workflows.template.message-text");
    case "message-was-sent":
      return i18n.t("workflows.template.message-was-sent");
    case "reaction-emoji":
      return i18n.t("workflows.template.reaction-emoji");
    case "reaction-was-added":
      return i18n.t("workflows.template.reaction-was-added");
    case "seconds-elapsed":
      return i18n.t("workflows.template.seconds-elapsed");
    case "sent-message-id":
      return i18n.t("workflows.template.sent-message-id");
    case "unix-timestamp":
      return i18n.t("workflows.template.unix-timestamp");
    case "workflow-channel-uuid":
      return i18n.t("workflows.template.workflow-channel-uuid");
  }
}

export function templateVariableGroupLabel(
  group: WorkflowTemplateVariableGroup,
): string {
  return group === "trigger"
    ? i18n.t("workflows.template.group-trigger")
    : i18n.t("workflows.template.group-previous-steps");
}

const TRIGGER_GROUP = "trigger" as const;
const PREVIOUS_STEPS_GROUP = "previous-steps" as const;

type TemplateVariableSpec = {
  caption: TemplateVariableCaption;
  group: WorkflowTemplateVariableGroup;
  value: string;
};

const COMMON_EVENT_VARIABLES: TemplateVariableSpec[] = [
  { value: "trigger.author", caption: "author-pubkey", group: TRIGGER_GROUP },
  {
    value: "trigger.channel_id",
    caption: "channel-uuid",
    group: TRIGGER_GROUP,
  },
  {
    value: "trigger.timestamp",
    caption: "unix-timestamp",
    group: TRIGGER_GROUP,
  },
  {
    value: "trigger.message_id",
    caption: "message-event-id",
    group: TRIGGER_GROUP,
  },
];

const MESSAGE_EVENT_VARIABLES: TemplateVariableSpec[] = [
  { value: "trigger.text", caption: "message-text", group: TRIGGER_GROUP },
  ...COMMON_EVENT_VARIABLES,
];

const DIFF_EVENT_VARIABLES: TemplateVariableSpec[] = [
  { value: "trigger.text", caption: "diff-text", group: TRIGGER_GROUP },
  ...COMMON_EVENT_VARIABLES,
];

const REACTION_EVENT_VARIABLES: TemplateVariableSpec[] = [
  { value: "trigger.emoji", caption: "reaction-emoji", group: TRIGGER_GROUP },
  ...COMMON_EVENT_VARIABLES,
];

const SCHEDULE_VARIABLES: TemplateVariableSpec[] = [
  {
    value: "trigger.channel_id",
    caption: "channel-uuid",
    group: TRIGGER_GROUP,
  },
  {
    value: "trigger.timestamp",
    caption: "unix-timestamp",
    group: TRIGGER_GROUP,
  },
];

const WEBHOOK_VARIABLES: TemplateVariableSpec[] = [
  {
    value: "trigger.channel_id",
    caption: "workflow-channel-uuid",
    group: TRIGGER_GROUP,
  },
];

/** Output fields each action publishes, keyed by action id. */
const STEP_OUTPUTS: Partial<
  Record<ActionType, Array<{ caption: TemplateVariableCaption; field: string }>>
> = {
  delay: [{ field: "slept_secs", caption: "seconds-elapsed" }],
  send_message: [
    { field: "sent", caption: "message-was-sent" },
    { field: "event_id", caption: "sent-message-id" },
  ],
  call_webhook: [
    { field: "status", caption: "http-response-status" },
    { field: "body", caption: "http-response-body" },
  ],
  add_reaction: [{ field: "added", caption: "reaction-was-added" }],
};

function triggerVariables(
  triggerType: TriggerType,
): WorkflowTemplateVariable[] {
  const specs =
    triggerType === "message_posted"
      ? MESSAGE_EVENT_VARIABLES
      : triggerType === "diff_posted"
        ? DIFF_EVENT_VARIABLES
        : triggerType === "reaction_added"
          ? REACTION_EVENT_VARIABLES
          : triggerType === "schedule"
            ? SCHEDULE_VARIABLES
            : WEBHOOK_VARIABLES;
  return specs.map(resolveVariable);
}

function resolveVariable(spec: TemplateVariableSpec): WorkflowTemplateVariable {
  return {
    description: templateVariableCaption(spec.caption),
    group: spec.group,
    groupLabel: templateVariableGroupLabel(spec.group),
    value: spec.value,
  };
}

export function workflowTemplateVariables(
  triggerType: TriggerType,
  previousSteps: StepFormState[],
): WorkflowTemplateVariable[] {
  const priorOutputs = previousSteps.flatMap((step) => {
    const outputs = STEP_OUTPUTS[step.action];
    if (!outputs || !/^[A-Za-z0-9_]+$/.test(step.id)) return [];
    return outputs.map(({ caption, field }) =>
      resolveVariable({
        caption,
        group: PREVIOUS_STEPS_GROUP,
        value: `steps.${step.id}.output.${field}`,
      }),
    );
  });

  return [...triggerVariables(triggerType), ...priorOutputs];
}

/** Find an unfinished `{{variable` token immediately before the caret. */
export function activeTemplateToken(
  value: string,
  caret: number,
): ActiveTemplateToken | null {
  const beforeCaret = value.slice(0, caret);
  const start = beforeCaret.lastIndexOf("{{");
  if (start < 0) return null;

  const query = beforeCaret.slice(start + 2);
  if (query.includes("}}") || !/^[A-Za-z0-9._]*$/.test(query)) return null;

  // If the caret is inside an existing template, replace the complete token
  // instead of leaving its old suffix behind after a suggestion is chosen.
  const closingBraces = value.indexOf("}}", caret);
  const nestedOpening = value.indexOf("{{", start + 2);
  const end =
    closingBraces >= 0 && (nestedOpening < 0 || closingBraces < nestedOpening)
      ? closingBraces + 2
      : caret;
  return { start, end, query };
}

export function insertTemplateVariable(
  value: string,
  token: ActiveTemplateToken,
  variable: string,
): { caret: number; value: string } {
  const insertion = `{{${variable}}}`;
  return {
    value: `${value.slice(0, token.start)}${insertion}${value.slice(token.end)}`,
    caret: token.start + insertion.length,
  };
}
