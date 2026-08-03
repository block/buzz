import type { ReplyPlacementMode } from "@/shared/api/types";
import { PersonaDropdownField } from "./PersonaDropdownField";
import type { PersonaDropdownOption } from "./agentConfigOptions";

const REPLY_PLACEMENT_OPTIONS: PersonaDropdownOption[] = [
  {
    label: "Follow the message scope",
    value: "follow-scope",
  },
  {
    label: "Always reply in a thread",
    value: "thread",
  },
  {
    label: "Always post at channel root",
    value: "top-level",
  },
];

const INHERIT_VALUE = "__inherit__";

function descriptionFor(
  mode: ReplyPlacementMode | null,
  effective?: ReplyPlacementMode,
) {
  if (mode === null) {
    return effective
      ? `Uses the inherited setting (${effective}). If no inherited setting is saved, Buzz keeps the historical thread behavior.`
      : "Uses the persona or global setting. If neither is saved, Buzz keeps the historical thread behavior.";
  }
  switch (mode) {
    case "follow-scope":
      return "A channel message gets a channel-root answer; a threaded message stays under that thread's root.";
    case "thread":
      return "Every human-facing answer uses a thread, preserving Buzz's historical behavior.";
    case "top-level":
      return "Every human-facing answer is posted at the channel root, including replies to threads.";
  }
}

/** Plain-English reply placement control shared by persona and agent editors. */
export function ReplyPlacementField({
  disabled,
  effectiveValue,
  inheritLabel = "Use persona / global default",
  onChange,
  value,
  allowInherit = false,
}: {
  disabled?: boolean;
  /** Effective value shown in the inherit explanation. */
  effectiveValue?: ReplyPlacementMode;
  /** Context-specific label for the inherited option. */
  inheritLabel?: string;
  onChange: (value: ReplyPlacementMode | null) => void;
  /** `null` means inherit when `allowInherit` is enabled. */
  value: ReplyPlacementMode | null;
  allowInherit?: boolean;
}) {
  const options = allowInherit
    ? [
        {
          label: inheritLabel,
          value: INHERIT_VALUE,
        },
        ...REPLY_PLACEMENT_OPTIONS,
      ]
    : REPLY_PLACEMENT_OPTIONS;
  const selectedValue = value ?? (allowInherit ? INHERIT_VALUE : "thread");

  return (
    <div className="space-y-2" data-testid="agent-reply-placement">
      <label
        className="text-sm font-medium text-foreground"
        htmlFor="agent-reply-placement-select"
      >
        How replies are placed
      </label>
      <PersonaDropdownField
        disabled={disabled}
        id="agent-reply-placement-select"
        onValueChange={(next) =>
          onChange(next === INHERIT_VALUE ? null : (next as ReplyPlacementMode))
        }
        options={options}
        placeholder="Follow the message scope"
        value={selectedValue}
      />
      <p className="text-xs leading-5 text-muted-foreground">
        {descriptionFor(value, effectiveValue)}
      </p>
    </div>
  );
}
