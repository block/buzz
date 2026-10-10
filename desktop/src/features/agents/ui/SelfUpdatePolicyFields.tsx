import {
  SELF_UPDATE_FIELDS,
  selfUpdateFieldLabel,
  toggleSelfUpdateField,
} from "../agentManagement";
import type { SelfUpdateField } from "@/shared/api/types";

/**
 * Per-agent self-update policy (#6287): which definition fields this agent may
 * change on its own `buzz agents draft-update` without the owner reviewing a
 * form. Everything unchecked stays on today's review path.
 */
export function SelfUpdatePolicyFields({
  disabled,
  value,
  onChange,
}: {
  disabled: boolean;
  value: readonly SelfUpdateField[];
  onChange: (value: SelfUpdateField[]) => void;
}) {
  function toggle(field: SelfUpdateField, checked: boolean) {
    onChange(toggleSelfUpdateField(value, field, checked));
  }

  return (
    <fieldset className="space-y-1.5" data-testid="edit-agent-self-update">
      <legend className="text-sm font-medium text-foreground">
        Allow this agent to update its own
      </legend>
      <div className="flex flex-wrap gap-x-4 gap-y-1.5">
        {SELF_UPDATE_FIELDS.map((field) => {
          const id = `edit-agent-self-update-${field}`;
          return (
            <label
              className="flex items-center gap-2 text-sm"
              htmlFor={id}
              key={field}
            >
              <input
                checked={value.includes(field)}
                disabled={disabled}
                id={id}
                onChange={(event) => toggle(field, event.target.checked)}
                type="checkbox"
              />
              {capitalize(selfUpdateFieldLabel(field))}
            </label>
          );
        })}
      </div>
      <p className="text-xs text-muted-foreground">
        {value.length === 0
          ? "Every draft-update this agent sends opens a review form; nothing changes until you save it."
          : "A draft-update from this agent that only touches the checked fields is applied without review and the agent restarts under its auto-restart setting. Every other agent sharing this definition must allow the same fields, since the edit reaches all of them. Anything else still opens the review form."}
      </p>
    </fieldset>
  );
}

function capitalize(label: string): string {
  return label.charAt(0).toUpperCase() + label.slice(1);
}
