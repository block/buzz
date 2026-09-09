import type { ComponentProps } from "react";
import { Progress as Primitive } from "@base-ui/react/progress";
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

function StyledValue({
  className,
  ...props
}: ComponentProps<typeof Primitive.Value>) {
  return (
    <Primitive.Value
      {...props}
      className={skin("bui-description", className)}
    />
  );
}

function StyledTrack({
  className,
  ...props
}: ComponentProps<typeof Primitive.Track>) {
  return (
    <Primitive.Track
      {...props}
      className={skin("bui-progress-track", className)}
    />
  );
}

function StyledIndicator({
  className,
  ...props
}: ComponentProps<typeof Primitive.Indicator>) {
  return (
    <Primitive.Indicator
      {...props}
      className={skin("bui-progress-indicator", className)}
    />
  );
}

/** Progress behavior primitives with the Buzz visual roles. */
export const Progress = {
  Root: StyledRoot,
  Label: StyledLabel,
  Value: StyledValue,
  Track: StyledTrack,
  Indicator: StyledIndicator,
};
