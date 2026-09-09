import type { ComponentProps } from "react";
import { Menu as Primitive } from "@base-ui/react/menu";
import { skin } from "../classes";

function StyledPositioner({
  className,
  ...props
}: ComponentProps<typeof Primitive.Positioner>) {
  return (
    <Primitive.Positioner
      {...props}
      className={skin("bui-positioner", className)}
    />
  );
}

function StyledPopup({
  className,
  ...props
}: ComponentProps<typeof Primitive.Popup>) {
  return (
    <Primitive.Popup {...props} className={skin("bui-popup", className)} />
  );
}

function StyledItem({
  className,
  ...props
}: ComponentProps<typeof Primitive.Item>) {
  return (
    <Primitive.Item {...props} className={skin("bui-option", className)} />
  );
}

function StyledSeparator({
  className,
  ...props
}: ComponentProps<typeof Primitive.Separator>) {
  return (
    <Primitive.Separator
      {...props}
      className={skin("bui-separator", className)}
    />
  );
}

function StyledGroup({
  className,
  ...props
}: ComponentProps<typeof Primitive.Group>) {
  return (
    <Primitive.Group {...props} className={skin("bui-stack", className)} />
  );
}

function StyledGroupLabel({
  className,
  ...props
}: ComponentProps<typeof Primitive.GroupLabel>) {
  return (
    <Primitive.GroupLabel
      {...props}
      className={skin("bui-group-label", className)}
    />
  );
}

function StyledCheckboxItem({
  className,
  ...props
}: ComponentProps<typeof Primitive.CheckboxItem>) {
  return (
    <Primitive.CheckboxItem
      {...props}
      className={skin("bui-option", className)}
    />
  );
}

function StyledCheckboxItemIndicator({
  className,
  ...props
}: ComponentProps<typeof Primitive.CheckboxItemIndicator>) {
  return (
    <Primitive.CheckboxItemIndicator
      {...props}
      className={skin("bui-option-check", className)}
    />
  );
}

function StyledRadioGroup({
  className,
  ...props
}: ComponentProps<typeof Primitive.RadioGroup>) {
  return (
    <Primitive.RadioGroup {...props} className={skin("bui-stack", className)} />
  );
}

function StyledRadioItem({
  className,
  ...props
}: ComponentProps<typeof Primitive.RadioItem>) {
  return (
    <Primitive.RadioItem {...props} className={skin("bui-option", className)} />
  );
}

function StyledRadioItemIndicator({
  className,
  ...props
}: ComponentProps<typeof Primitive.RadioItemIndicator>) {
  return (
    <Primitive.RadioItemIndicator
      {...props}
      className={skin("bui-option-check", className)}
    />
  );
}

function StyledSubmenuTrigger({
  className,
  ...props
}: ComponentProps<typeof Primitive.SubmenuTrigger>) {
  return (
    <Primitive.SubmenuTrigger
      {...props}
      className={skin("bui-option", className)}
    />
  );
}

/** Menu behavior primitives with the Buzz visual roles. */
export const Menu = {
  Root: Primitive.Root,
  Trigger: Primitive.Trigger,
  Portal: Primitive.Portal,
  Positioner: StyledPositioner,
  Popup: StyledPopup,
  Item: StyledItem,
  Separator: StyledSeparator,
  Group: StyledGroup,
  GroupLabel: StyledGroupLabel,
  CheckboxItem: StyledCheckboxItem,
  CheckboxItemIndicator: StyledCheckboxItemIndicator,
  RadioGroup: StyledRadioGroup,
  RadioItem: StyledRadioItem,
  RadioItemIndicator: StyledRadioItemIndicator,
  SubmenuRoot: Primitive.SubmenuRoot,
  SubmenuTrigger: StyledSubmenuTrigger,
};
