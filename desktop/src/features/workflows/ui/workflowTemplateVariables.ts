import type {
  ActionType,
  StepFormState,
  TriggerType,
} from "./workflowFormTypes";

export type WorkflowTemplateVariable = {
  description: string;
  group: "Trigger" | "Previous steps";
  value: string;
};

export type ActiveTemplateToken = {
  end: number;
  query: string;
  start: number;
};

const COMMON_EVENT_VARIABLES: WorkflowTemplateVariable[] = [
  {
    value: "trigger.author",
    description: "Author pubkey",
    group: "Trigger",
  },
  {
    value: "trigger.channel_id",
    description: "Channel UUID",
    group: "Trigger",
  },
  {
    value: "trigger.timestamp",
    description: "Unix timestamp",
    group: "Trigger",
  },
  {
    value: "trigger.message_id",
    description: "Message event ID",
    group: "Trigger",
  },
];

const STEP_OUTPUTS: Partial<Record<ActionType, Record<string, string>>> = {
  delay: { slept_secs: "Seconds elapsed" },
  send_message: {
    sent: "Whether the message was sent",
    event_id: "Sent message ID",
  },
  call_webhook: { status: "HTTP response status", body: "HTTP response body" },
  add_reaction: {
    added: "Whether the reaction was added",
  },
};

function triggerVariables(
  triggerType: TriggerType,
): WorkflowTemplateVariable[] {
  switch (triggerType) {
    case "message_posted":
      return [
        {
          value: "trigger.text",
          description: "Message text",
          group: "Trigger",
        },
        ...COMMON_EVENT_VARIABLES,
      ];
    case "diff_posted":
      return [
        { value: "trigger.text", description: "Diff text", group: "Trigger" },
        ...COMMON_EVENT_VARIABLES,
      ];
    case "reaction_added":
      return [
        {
          value: "trigger.emoji",
          description: "Reaction emoji",
          group: "Trigger",
        },
        ...COMMON_EVENT_VARIABLES,
      ];
    case "schedule":
      return COMMON_EVENT_VARIABLES.filter(
        ({ value }) =>
          value === "trigger.channel_id" || value === "trigger.timestamp",
      );
    case "webhook":
      return [
        {
          value: "trigger.channel_id",
          description: "Workflow channel UUID",
          group: "Trigger",
        },
      ];
  }
}

export function workflowTemplateVariables(
  triggerType: TriggerType,
  previousSteps: StepFormState[],
): WorkflowTemplateVariable[] {
  const priorOutputs = previousSteps.flatMap((step) => {
    const outputs = STEP_OUTPUTS[step.action];
    if (!outputs || !/^[A-Za-z0-9_]+$/.test(step.id)) return [];
    return Object.entries(outputs).map(([field, description]) => ({
      value: `steps.${step.id}.output.${field}`,
      description,
      group: "Previous steps" as const,
    }));
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
