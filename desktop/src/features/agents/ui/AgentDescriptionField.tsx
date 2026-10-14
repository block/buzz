import { cn } from "@/shared/lib/cn";
import { Input } from "@/shared/ui/input";
import {
  agentDescriptionCharacterCount,
  clampAgentDescription,
  MAX_AGENT_DESCRIPTION_CHARS,
} from "../lib/agentDescription";
import {
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
  PERSONA_LABEL_OPTIONAL_CLASS,
} from "./agentConfigOptions";

/** Show the live character counter once within this many chars of the cap. */
const COUNTER_VISIBLE_WITHIN = 40;

type AgentIdentityFieldsProps = {
  displayName: string;
  onDisplayNameChange: (value: string) => void;
  description: string;
  onDescriptionChange: (value: string) => void;
  disabled: boolean;
};

/**
 * The persona dialog's identity block: the required "Agent name" input and
 * the optional public "Description" input directly beneath it.
 *
 * The hard 280-Unicode-scalar cap mirrors Rust's `chars().count()`, with a
 * live counter once the value approaches the limit.
 */
export function AgentIdentityFields({
  displayName,
  onDisplayNameChange,
  description,
  onDescriptionChange,
  disabled,
}: AgentIdentityFieldsProps) {
  const placeholder = "What this agent does, in a sentence";
  const descriptionLength = agentDescriptionCharacterCount(description);
  const showCounter =
    descriptionLength >= MAX_AGENT_DESCRIPTION_CHARS - COUNTER_VISIBLE_WITHIN;

  return (
    <>
      <div className="space-y-1.5">
        <label
          className="text-sm font-medium text-foreground"
          htmlFor="persona-display-name"
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
            id="persona-display-name"
            onChange={(event) => onDisplayNameChange(event.target.value)}
            placeholder="Fizz"
            value={displayName}
          />
        </div>
      </div>

      <div className="space-y-1.5">
        <label
          className="text-sm font-medium text-foreground"
          htmlFor="persona-description"
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
            disabled={disabled}
            id="persona-description"
            onChange={(event) =>
              onDescriptionChange(clampAgentDescription(event.target.value))
            }
            placeholder={placeholder}
            value={description}
          />
        </div>
        <div className="flex items-baseline justify-between gap-2">
          <p className="text-xs text-muted-foreground">
            Shown publicly on the agent&apos;s card and profile.
          </p>
          {showCounter ? (
            <span className="shrink-0 text-2xs tabular-nums text-muted-foreground">
              {descriptionLength}/{MAX_AGENT_DESCRIPTION_CHARS}
            </span>
          ) : null}
        </div>
      </div>
    </>
  );
}
