// Tests for the popover trigger's focus-ownership decision. The wrapper should
// carry its own role/tabIndex only when the panel can open and the child is not
// already a focusable control; otherwise two nested Tab stops land on one
// avatar (block/buzz#2394). No jsdom here, so the decision is a pure predicate
// over the child's { type, props } shape and is tested directly.

import test from "node:test";
import assert from "node:assert/strict";

import {
  isInteractiveTriggerChild,
  triggerShouldOwnFocus,
} from "./userProfilePopoverTrigger.ts";

const button = { type: "button", props: {} };
const span = { type: "span", props: {} };

test("triggerShouldOwnFocus_defersToInteractiveChild", () => {
  assert.equal(triggerShouldOwnFocus(button, true), false);
});

test("triggerShouldOwnFocus_keepsFocusForNonInteractiveChild", () => {
  // SystemMessageRow mention chips pass a span, so the wrapper is the only Tab
  // stop and must keep owning focus.
  assert.equal(triggerShouldOwnFocus(span, true), true);
});

test("triggerShouldOwnFocus_neverOwnsFocusWhenPanelCannotOpen", () => {
  assert.equal(triggerShouldOwnFocus(button, false), false);
  assert.equal(triggerShouldOwnFocus(span, false), false);
});

test("isInteractiveTriggerChild_button", () => {
  assert.equal(isInteractiveTriggerChild(button), true);
});

test("isInteractiveTriggerChild_span", () => {
  assert.equal(isInteractiveTriggerChild(span), false);
});

test("isInteractiveTriggerChild_selectAndTextarea", () => {
  assert.equal(isInteractiveTriggerChild({ type: "select", props: {} }), true);
  assert.equal(
    isInteractiveTriggerChild({ type: "textarea", props: {} }),
    true,
  );
});

test("isInteractiveTriggerChild_anchorNeedsHref", () => {
  assert.equal(
    isInteractiveTriggerChild({ type: "a", props: { href: "#" } }),
    true,
  );
  assert.equal(isInteractiveTriggerChild({ type: "a", props: {} }), false);
});

test("isInteractiveTriggerChild_inputExceptHidden", () => {
  assert.equal(isInteractiveTriggerChild({ type: "input", props: {} }), true);
  assert.equal(
    isInteractiveTriggerChild({ type: "input", props: { type: "hidden" } }),
    false,
  );
});

test("isInteractiveTriggerChild_disabledControlIsNotFocusable", () => {
  // A disabled control is not in the tab order, so the wrapper must keep focus
  // rather than defer to a child that cannot take it.
  assert.equal(
    isInteractiveTriggerChild({ type: "button", props: { disabled: true } }),
    false,
  );
  assert.equal(
    isInteractiveTriggerChild({ type: "input", props: { disabled: true } }),
    false,
  );
});

test("isInteractiveTriggerChild_tabIndexOverridesElementType", () => {
  // An explicit tabIndex decides focusability for any element.
  assert.equal(
    isInteractiveTriggerChild({ type: "span", props: { tabIndex: 0 } }),
    true,
  );
  assert.equal(
    isInteractiveTriggerChild({ type: "button", props: { tabIndex: -1 } }),
    false,
  );
});

test("isInteractiveTriggerChild_componentChildIsNotInteractive", () => {
  // A component child cannot be introspected, so the wrapper keeps focus (the
  // safe default that preserves prior behavior).
  const Component = () => null;
  assert.equal(
    isInteractiveTriggerChild({ type: Component, props: {} }),
    false,
  );
});

test("isInteractiveTriggerChild_missingChild", () => {
  assert.equal(isInteractiveTriggerChild(null), false);
  assert.equal(isInteractiveTriggerChild(undefined), false);
});
