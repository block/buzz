import { Accordion as BaseAccordion } from "@base-ui/react/accordion";
import { IconChevronDown } from "@tabler/icons-react";
import type { ReactNode } from "react";

/** Accessible disclosure groups. Base UI owns expansion and keyboard behavior. */
export function Accordion({
  items,
  defaultValue = [],
  value,
  onValueChange,
  variant = "default",
  headingLevel = 3,
}: {
  items: readonly { value: string; title: ReactNode; content: ReactNode }[];
  defaultValue?: string[];
  value?: string[];
  onValueChange?: (value: string[]) => void;
  variant?: "default" | "navigation" | "activity";
  headingLevel?: 2 | 3;
}) {
  return (
    <BaseAccordion.Root
      className="buzz-accordion"
      data-variant={variant}
      defaultValue={defaultValue}
      value={value}
      onValueChange={onValueChange}
      multiple
    >
      {items.map((item) => (
        <BaseAccordion.Item key={item.value} value={item.value}>
          <BaseAccordion.Header
            className="buzz-accordion-heading"
            render={(props) =>
              headingLevel === 2 ? (
                <h2 {...props}>{props.children}</h2>
              ) : (
                <h3 {...props}>{props.children}</h3>
              )
            }
          >
            <BaseAccordion.Trigger className="buzz-accordion-trigger text-body">
              <span>{item.title}</span>
              <IconChevronDown size={14} aria-hidden="true" />
            </BaseAccordion.Trigger>
          </BaseAccordion.Header>
          <BaseAccordion.Panel className="buzz-accordion-panel">
            {item.content}
          </BaseAccordion.Panel>
        </BaseAccordion.Item>
      ))}
    </BaseAccordion.Root>
  );
}
