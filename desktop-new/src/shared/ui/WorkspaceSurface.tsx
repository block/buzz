import type { ComponentProps, ElementType, ReactNode } from "react";

type SurfaceVariant =
  | "panel"
  | "connected-left"
  | "connected-right"
  | "navigator-demo";

type WorkspaceSurfaceProps<T extends ElementType> = {
  as?: T;
  children: ReactNode;
  variant?: SurfaceVariant;
} & Omit<ComponentProps<T>, "as" | "children" | "className">;

export function WorkspaceSurface<T extends ElementType = "section">({
  as,
  children,
  variant = "panel",
  ...props
}: WorkspaceSurfaceProps<T>) {
  const Component = as ?? "section";
  return (
    <Component {...props} className="workspace-surface" data-variant={variant}>
      {children}
    </Component>
  );
}
