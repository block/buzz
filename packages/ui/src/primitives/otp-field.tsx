import type { ComponentProps } from "react";
import { OTPField as Primitive } from "@base-ui/react/otp-field";
import { skin } from "../classes";

function StyledRoot({
  className,
  ...props
}: ComponentProps<typeof Primitive.Root>) {
  return <Primitive.Root {...props} className={skin("bui-otp", className)} />;
}

function StyledInput({
  className,
  ...props
}: ComponentProps<typeof Primitive.Input>) {
  return (
    <Primitive.Input {...props} className={skin("bui-otp-input", className)} />
  );
}

/** OTPField behavior primitives with the Buzz visual roles. */
export const OTPField = { Root: StyledRoot, Input: StyledInput };
