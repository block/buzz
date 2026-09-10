import { Select as BaseSelect } from "@base-ui/react/select";
import { IconCheck, IconChevronDown } from "@tabler/icons-react";
import { Button } from "./Button";

export type SelectGroup = {
  label: string;
  options: readonly { value: string; label: string }[];
};

/** A compact, labelled single-choice control with accessible option groups. */
export function Select({
  label,
  value,
  groups,
  onValueChange,
}: {
  label: string;
  value: string;
  groups: readonly SelectGroup[];
  onValueChange: (value: string) => void;
}) {
  return (
    <div className="buzz-select text-body">
      <BaseSelect.Root
        value={value}
        items={groups.flatMap((group) => group.options)}
        onValueChange={(next) => {
          if (next !== null) onValueChange(next);
        }}
      >
        <BaseSelect.Label className="text-primary">{label}</BaseSelect.Label>
        <BaseSelect.Trigger
          render={(props) => (
            <Button {...props} variant="ghost">
              <BaseSelect.Value />
              <BaseSelect.Icon>
                <IconChevronDown size={14} aria-hidden="true" />
              </BaseSelect.Icon>
            </Button>
          )}
        />
        <BaseSelect.Portal>
          <BaseSelect.Positioner
            sideOffset={4}
            align="start"
            alignItemWithTrigger={false}
          >
            <BaseSelect.Popup className="buzz-select-popup text-body-sm">
              <BaseSelect.List>
                {groups.map((group) => (
                  <BaseSelect.Group key={group.label}>
                    <BaseSelect.GroupLabel className="buzz-select-group-label text-secondary">
                      {group.label}
                    </BaseSelect.GroupLabel>
                    {group.options.map((option) => (
                      <BaseSelect.Item
                        key={option.value}
                        value={option.value}
                        className="buzz-select-option"
                      >
                        <BaseSelect.ItemText>
                          {option.label}
                        </BaseSelect.ItemText>
                        <BaseSelect.ItemIndicator>
                          <IconCheck size={14} aria-hidden="true" />
                        </BaseSelect.ItemIndicator>
                      </BaseSelect.Item>
                    ))}
                  </BaseSelect.Group>
                ))}
              </BaseSelect.List>
            </BaseSelect.Popup>
          </BaseSelect.Positioner>
        </BaseSelect.Portal>
      </BaseSelect.Root>
    </div>
  );
}
