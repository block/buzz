import type { ComponentProps } from "react";
import { Slider as Primitive } from "@base-ui/react/slider";
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

function StyledControl({
  className,
  ...props
}: ComponentProps<typeof Primitive.Control>) {
  return (
    <Primitive.Control
      {...props}
      className={skin("bui-slider-control", className)}
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
      className={skin("bui-slider-track", className)}
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
      className={skin("bui-slider-indicator", className)}
    />
  );
}

function StyledThumb({
  className,
  ...props
}: ComponentProps<typeof Primitive.Thumb>) {
  return (
    <Primitive.Thumb
      {...props}
      className={skin("bui-slider-thumb", className)}
    />
  );
}

/** Slider behavior primitives with the Buzz visual roles. */
export const Slider = {
  Root: StyledRoot,
  Label: StyledLabel,
  Value: StyledValue,
  Control: StyledControl,
  Track: StyledTrack,
  Indicator: StyledIndicator,
  Thumb: StyledThumb,
};
