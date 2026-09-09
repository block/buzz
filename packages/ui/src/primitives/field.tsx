import type { ComponentProps } from "react";
import { Field as Primitive } from "@base-ui/react/field";
import { skin } from "../classes";

function StyledRoot({
  className,
  ...props
}: ComponentProps<typeof Primitive.Root>) {
  return <Primitive.Root {...props} className={skin("bui-field", className)} />;
}

function StyledLabel({
  className,
  ...props
}: ComponentProps<typeof Primitive.Label>) {
  return (
    <Primitive.Label {...props} className={skin("bui-label", className)} />
  );
}

function StyledDescription({
  className,
  ...props
}: ComponentProps<typeof Primitive.Description>) {
  return (
    <Primitive.Description
      {...props}
      className={skin("bui-description", className)}
    />
  );
}

function StyledError({
  className,
  ...props
}: ComponentProps<typeof Primitive.Error>) {
  return (
    <Primitive.Error
      {...props}
      className={skin("bui-field-error", className)}
    />
  );
}

function StyledControl({
  className,
  ...props
}: ComponentProps<typeof Primitive.Control>) {
  return (
    <Primitive.Control {...props} className={skin("bui-input", className)} />
  );
}

/** Field behavior primitives with the Buzz visual roles. */
export const Field = {
  Root: StyledRoot,
  Label: StyledLabel,
  Description: StyledDescription,
  Error: StyledError,
  Control: StyledControl,
};
