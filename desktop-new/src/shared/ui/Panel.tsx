import type { ComponentProps, ElementType, ReactNode } from "react";

type PanelVariant =
  | "panel"
  | "connected-left"
  | "connected-right"
  | "navigator-demo";

type PanelProps<T extends ElementType> = {
  as?: T;
  children: ReactNode;
  variant?: PanelVariant;
} & Omit<ComponentProps<T>, "as" | "children" | "className">;

export function Panel<T extends ElementType = "section">({
  as,
  children,
  variant = "panel",
  ...props
}: PanelProps<T>) {
  const Component = as ?? "section";
  return (
    <Component {...props} className="panel" data-variant={variant}>
      {children}
    </Component>
  );
}
