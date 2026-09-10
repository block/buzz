import { Tabs as BaseTabs } from "@base-ui/react/tabs";
import type { ReactNode } from "react";

export type TabItem<Value extends string> = {
  value: Value;
  label: string;
  icon?: ReactNode;
};

/**
 * Which background this control is drawn for.
 *
 * `chrome` is the glass pill that belongs on the app gradient. `panel` is an
 * underline for a plain light or dark surface.
 *
 * **A variant rather than a second component**, because behaviour, keyboard
 * model, accessibility, props, and the Base UI tabs underneath are identical —
 * only the appearance differs, which is what a variant is for. `IconButton`
 * spells the same idea the same way.
 *
 * `chrome` is the default because every existing call site is on the gradient.
 */
export type TabsVariant = "chrome" | "panel";

export function Tabs<Value extends string>({
  value,
  items,
  label,
  onValueChange,
  trailingAction,
  variant = "chrome",
}: {
  value: Value;
  items: readonly TabItem<Value>[];
  label: string;
  onValueChange: (value: Value) => void;
  trailingAction?: ReactNode;
  variant?: TabsVariant;
}) {
  return (
    <BaseTabs.Root
      className="buzz-tabs"
      data-variant={variant}
      value={value}
      onValueChange={(nextValue) => onValueChange(nextValue as Value)}
    >
      <BaseTabs.List className="buzz-tabs-list" aria-label={label}>
        <BaseTabs.Indicator className="buzz-tabs-indicator" />
        {items.map((item) => (
          <BaseTabs.Tab
            key={item.value}
            value={item.value}
            className="buzz-tabs-tab"
            /* Base UI spells the selected tab `data-active`; every other
               selectable thing in this codebase is styled on `data-selected`.
               Restating it here keeps one spelling in the stylesheet rather than
               asking a reader to know which components happen to be Base
               UI-backed. Without it the selected label silently kept
               `--text-secondary` — no error, just a tab that never looked
               selected. */
            data-selected={item.value === value || undefined}
          >
            {item.icon}
            <span>{item.label}</span>
          </BaseTabs.Tab>
        ))}
      </BaseTabs.List>
      {trailingAction}
    </BaseTabs.Root>
  );
}
