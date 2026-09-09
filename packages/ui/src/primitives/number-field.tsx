import type { ComponentProps } from "react";
import { NumberField as Primitive } from "@base-ui/react/number-field";
import { skin } from "../classes";

function StyledRoot({
  className,
  ...props
}: ComponentProps<typeof Primitive.Root>) {
  return <Primitive.Root {...props} className={skin("bui-field", className)} />;
}

function StyledGroup({
  className,
  ...props
}: ComponentProps<typeof Primitive.Group>) {
  return (
    <Primitive.Group
      {...props}
      className={skin("bui-number-group", className)}
    />
  );
}

function StyledInput({
  className,
  ...props
}: ComponentProps<typeof Primitive.Input>) {
  return (
    <Primitive.Input
      {...props}
      className={skin("bui-number-input", className)}
    />
  );
}

function StyledIncrement({
  className,
  ...props
}: ComponentProps<typeof Primitive.Increment>) {
  return (
    <Primitive.Increment
      {...props}
      className={skin("bui-icon-control", className)}
    />
  );
}

function StyledDecrement({
  className,
  ...props
}: ComponentProps<typeof Primitive.Decrement>) {
  return (
    <Primitive.Decrement
      {...props}
      className={skin("bui-icon-control", className)}
    />
  );
}

function StyledScrubArea({
  className,
  ...props
}: ComponentProps<typeof Primitive.ScrubArea>) {
  return (
    <Primitive.ScrubArea {...props} className={skin("bui-label", className)} />
  );
}

/** NumberField behavior primitives with the Buzz visual roles. */
export const NumberField = {
  Root: StyledRoot,
  Group: StyledGroup,
  Input: StyledInput,
  Increment: StyledIncrement,
  Decrement: StyledDecrement,
  ScrubArea: StyledScrubArea,
};
