import { cn } from "@/shared/lib/cn";
import { Input } from "@/shared/ui/input";
import { Textarea } from "@/shared/ui/textarea";
import { clampAgentDescription } from "../lib/agentDescription";
import {
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
  PERSONA_LABEL_OPTIONAL_CLASS,
} from "./agentConfigOptions";

/**
 * Name + (optional) public description + agent instructions on the instance
 * Edit dialog. Description is definition-owned and only shown when a linked
 * persona exists; instructions are editable for unlinked agents or editable
 * linked definitions.
 */
export function EditAgentIdentityFields({
  descriptionDraft,
  disabled,
  linkedPersonaPresent,
  name,
  onDescriptionChange,
  onNameChange,
  onSystemPromptChange,
  personaIdentityEditable,
  systemPrompt,
}: {
  descriptionDraft: string;
  disabled: boolean;
  linkedPersonaPresent: boolean;
  name: string;
  onDescriptionChange: (value: string) => void;
  onNameChange: (value: string) => void;
  onSystemPromptChange: (value: string) => void;
  personaIdentityEditable: boolean;
  systemPrompt: string;
}) {
  const instructionsReadOnly = linkedPersonaPresent && !personaIdentityEditable;

  return (
    <>
      <div className="space-y-1.5">
        <label
          className="text-sm font-medium text-foreground"
          htmlFor="edit-agent-name"
        >
          Agent name
        </label>
        <div
          className={cn(
            "flex min-h-11 items-center px-3",
            PERSONA_FIELD_SHELL_CLASS,
          )}
        >
          <Input
            autoCorrect="off"
            className={cn(
              "h-8 px-0 py-0 leading-6",
              PERSONA_FIELD_CONTROL_CLASS,
            )}
            disabled={disabled}
            id="edit-agent-name"
            onChange={(event) => onNameChange(event.target.value)}
            placeholder="Agent name"
            value={name}
          />
        </div>
      </div>

      {linkedPersonaPresent ? (
        <div className="space-y-1.5">
          <label
            className="text-sm font-medium text-foreground"
            htmlFor="edit-agent-description"
          >
            Description
            <span className={PERSONA_LABEL_OPTIONAL_CLASS}>Optional</span>
          </label>
          <div
            className={cn(
              "flex min-h-11 items-center px-3",
              PERSONA_FIELD_SHELL_CLASS,
            )}
          >
            <Input
              className={cn(
                "h-8 px-0 py-0 leading-6",
                PERSONA_FIELD_CONTROL_CLASS,
              )}
              disabled={disabled || !personaIdentityEditable}
              id="edit-agent-description"
              onChange={(event) =>
                onDescriptionChange(clampAgentDescription(event.target.value))
              }
              placeholder="What this agent does, in a sentence"
              value={descriptionDraft}
            />
          </div>
        </div>
      ) : null}

      <div className="space-y-1.5">
        <label
          className="text-sm font-medium text-foreground"
          htmlFor="edit-agent-system-prompt"
        >
          Agent instructions
        </label>
        <div className={PERSONA_FIELD_SHELL_CLASS}>
          <Textarea
            className={cn(
              "min-h-40 resize-y px-3 py-3 leading-5",
              PERSONA_FIELD_CONTROL_CLASS,
            )}
            disabled={disabled || instructionsReadOnly}
            id="edit-agent-system-prompt"
            onChange={(event) => onSystemPromptChange(event.target.value)}
            placeholder="Describe what this agent should do."
            value={systemPrompt}
          />
        </div>
        {instructionsReadOnly ? (
          <p className="text-xs text-muted-foreground">
            Instructions come from this agent&apos;s shared definition and
            can&apos;t be edited here.
          </p>
        ) : null}
      </div>
    </>
  );
}
