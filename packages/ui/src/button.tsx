import type { ComponentProps } from "react";
import { Button as Primitive } from "@base-ui/react/button";
import { LoaderCircle } from "lucide-react";
import { skin } from "./classes";
/** Appearance and behavior of an action. Loading preserves its accessible label. */
export type ButtonProps = ComponentProps<typeof Primitive> & {
  variant?: "primary" | "secondary" | "outline" | "ghost" | "danger" | "link";
  size?: "sm" | "md" | "lg";
  loading?: boolean;
};
/** A pill action with explicit variants, native keyboard semantics, and bounded loading feedback. */
export function Button({
  variant = "primary",
  size = "md",
  loading = false,
  disabled,
  className,
  children,
  ...props
}: ButtonProps) {
  return (
    <Primitive
      type="button"
      {...props}
      disabled={disabled || loading}
      aria-busy={loading || undefined}
      data-loading={loading || undefined}
      data-variant={variant}
      data-size={size}
      className={skin("bui-button", className)}
    >
      <span className="bui-button-label">{children}</span>
      {loading && <LoaderCircle className="bui-loading" aria-hidden="true" />}
    </Primitive>
  );
}
/** An icon-only action must have a readable accessible name. */
export function IconButton(props: ButtonProps & { "aria-label": string }) {
  return (
    <Button {...props} className={skin("bui-icon-button", props.className)} />
  );
}
