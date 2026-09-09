import type { ComponentProps } from "react";
import { Fieldset as Primitive } from "@base-ui/react/fieldset";
import { skin } from "../classes";

function StyledRoot({
  className,
  ...props
}: ComponentProps<typeof Primitive.Root>) {
  return (
    <Primitive.Root {...props} className={skin("bui-fieldset", className)} />
  );
}

function StyledLegend({
  className,
  ...props
}: ComponentProps<typeof Primitive.Legend>) {
  return (
    <Primitive.Legend {...props} className={skin("bui-label", className)} />
  );
}

/** Fieldset behavior primitives with the Buzz visual roles. */
export const Fieldset = { Root: StyledRoot, Legend: StyledLegend };
