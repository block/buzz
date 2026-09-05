import { Button as BaseButton } from "@base-ui/react/button";
import type { ComponentProps, ReactNode } from "react";

type ButtonVariant = "primary" | "quiet" | "ghost";
type ButtonSize = "compact" | "default";

export type ButtonProps = Omit<
  ComponentProps<typeof BaseButton>,
  "className"
> & {
  children: ReactNode;
  variant?: ButtonVariant;
  size?: ButtonSize;
};

export function Button({
  children,
  variant = "quiet",
  size = "default",
  type = "button",
  ...props
}: ButtonProps) {
  return (
    <BaseButton
      {...props}
      type={type}
      className="buzz-button"
      data-variant={variant}
      data-size={size}
    >
      {children}
    </BaseButton>
  );
}
