import { useId, useState } from "react";
import { Checkbox, Switch, Tabs } from "@buzz/ui";
import { Check, Minus } from "lucide-react";

function CheckboxSample({ label }: { label: string }) {
  const id = useId();
  const [checked, setChecked] = useState<boolean | "mixed">(
    label === "Indeterminate" ? "mixed" : label === "Checked",
  );
  return (
    <label htmlFor={id} className="bui-inline text-body">
      <Checkbox.Root
        id={id}
        checked={checked === true}
        indeterminate={checked === "mixed"}
        onCheckedChange={setChecked}
        disabled={label === "Disabled"}
      >
        <Checkbox.Indicator>
          {checked === "mixed" ? (
            <Minus aria-hidden="true" />
          ) : (
            <Check aria-hidden="true" />
          )}
        </Checkbox.Indicator>
      </Checkbox.Root>
      {label}
    </label>
  );
}

/** Independent, interactive check states with native disabled semantics. */
export function CheckboxStates() {
  return (
    <div className="component-state-grid">
      {["Unchecked", "Checked", "Indeterminate", "Disabled"].map((label) => (
        <CheckboxSample key={label} label={label} />
      ))}
    </div>
  );
}

/** Switch samples retain the same keyboard and pointer behavior as the catalog. */
export function SwitchStates() {
  const id = useId();
  return (
    <div className="component-state-grid">
      {["Off", "On", "Disabled off", "Disabled on"].map((state) => (
        <label
          key={state}
          htmlFor={`${id}-${state.replaceAll(" ", "-")}`}
          className="bui-inline text-body"
        >
          <Switch.Root
            id={`${id}-${state.replaceAll(" ", "-")}`}
            defaultChecked={state === "On" || state === "Disabled on"}
            disabled={state.startsWith("Disabled")}
          >
            <Switch.Thumb />
          </Switch.Root>
          {state}
        </label>
      ))}
    </div>
  );
}

/** Both supported tab materials and a disabled destination. */
export function TabsStates() {
  return (
    <div className="component-state-grid">
      {(["solid", "glass"] as const).map((variant) => (
        <section
          key={variant}
          className={
            variant === "glass" ? "bg-app rounded-container p-4" : "p-4"
          }
        >
          <h3 className="text-label mb-4">
            {variant === "glass" ? "Glass" : "Solid"}
          </h3>
          <Tabs.Root defaultValue="overview">
            <Tabs.List variant={variant} aria-label={`${variant} example`}>
              <Tabs.Tab value="overview">Overview</Tabs.Tab>
              <Tabs.Tab value="activity">Activity</Tabs.Tab>
              <Tabs.Tab value="files" disabled>
                Files
              </Tabs.Tab>
              <Tabs.Indicator />
            </Tabs.List>
            <Tabs.Panel value="overview">People building together.</Tabs.Panel>
            <Tabs.Panel value="activity">Three tasks completed.</Tabs.Panel>
          </Tabs.Root>
        </section>
      ))}
    </div>
  );
}
