import type { ComponentProps } from "react";
import { Input as Primitive } from "@base-ui/react/input";
import { Field } from "@base-ui/react/field";
import { skin } from "./classes";
/** A text input that participates in Field labeling and validation. */
export function Input({
  className,
  ...props
}: ComponentProps<typeof Primitive>) {
  return <Primitive {...props} className={skin("bui-input", className)} />;
}
/** A multiline field using the same labeling and validation seam as Input. */
export function Textarea({
  className,
  ...props
}: ComponentProps<typeof Field.Control>) {
  return (
    <Field.Control
      {...props}
      render={<textarea />}
      className={skin("bui-input bui-textarea", className)}
    />
  );
}
