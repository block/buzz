import { Tabs } from "@base-ui/react/tabs";
import type { ReactNode } from "react";

export type SegmentedNavigationItem<Value extends string> = {
  value: Value;
  label: string;
  icon?: ReactNode;
};

export function SegmentedNavigation<Value extends string>({
  value,
  items,
  label,
  onValueChange,
  trailingAction,
}: {
  value: Value;
  items: readonly SegmentedNavigationItem<Value>[];
  label: string;
  onValueChange: (value: Value) => void;
  trailingAction?: ReactNode;
}) {
  return (
    <Tabs.Root
      className="segmented-navigation"
      value={value}
      onValueChange={(nextValue) => onValueChange(nextValue as Value)}
    >
      <Tabs.List className="segmented-navigation-list" aria-label={label}>
        <Tabs.Indicator className="segmented-navigation-indicator" />
        {items.map((item) => (
          <Tabs.Tab
            key={item.value}
            value={item.value}
            className="segmented-navigation-item"
          >
            {item.icon}
            <span>{item.label}</span>
          </Tabs.Tab>
        ))}
      </Tabs.List>
      {trailingAction}
    </Tabs.Root>
  );
}
