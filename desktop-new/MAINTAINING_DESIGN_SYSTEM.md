# Maintaining the design system

## The point

The system helps Buzz stay clear and coherent as it grows. It should make the common choice easy, while leaving room for a design to be specific, surprising, or new. It is a shared memory of decisions that have been made—not a gate a designer needs to pass through.

When the system cannot express the right design, change the system. Do not distort the design to satisfy an old rule.

## Start with the work in front of you

Build the screen or interaction you are trying to make. Use the existing colors, type, surfaces, and components when they fit. Look at the result in context, not just in a token table.

A repeated need is evidence. One-off work is evidence too: it may reveal that a ramp step is wrong, a component needs another supported state, or a rule no longer reflects the product.

## Color: use the ramps first

Each color ramp has a version for light mode and a version for dark mode. A step has the same job in both: a quiet neutral surface, a selected row, a strong accent, or readable colored text. That lets a screen use a normal ramp step such as `neutral-2` and still work in both modes.

A semantic color name is earned when a plain ramp step cannot tell the truth:

- Light and dark need different steps for the same job. A panel, for example, needs a different relationship to its surroundings in each mode.
- A name protects a real rule. The three text levels and the single shared border weight are examples.
- A pattern has appeared enough times that giving it one name makes the product easier to understand and keep consistent.

Do not create a semantic name just to avoid choosing a ramp step. If a name points to the same step in both modes, it is usually hiding a decision rather than recording one.

## Refine the ramp when the product teaches us something

The ramps are not sacred. They began as a useful starting point and should change when real product work gives us better evidence.

The lighter search-field neutral is a good example. A real prototype showed that the old value was too heavy, so we corrected the neutral ramp instead of adding a search-only color or a fractional stop. That improvement now benefits every quiet compact surface that uses the same step.

Before adding a new step or a new role, ask:

1. Is an existing step already the right answer?
2. If not, is the ramp itself missing the right relationship?
3. Does light mode need one step and dark mode another? If so, give that relationship a semantic name.
4. Will people see this pattern again, or is it one moment in one screen?

## Components grow from real repetition

A shared component is a promise: the same interaction and visual language will work the same way wherever it appears.

Build the first version where it is needed. Once another real use appears, decide what is truly shared:

- a generic building block belongs in the design system;
- Buzz-specific behavior belongs with the product capability that owns it;
- a complete arrangement should only become reusable when another surface needs that exact arrangement.

Do not build a catalogue in advance. Small, proven components are more flexible than a large component with a long list of switches.

## Keep the important states visible

Every interactive piece should have a clear default, hover, selected state where it applies, and disabled state where it matters. Selection is a lasting statement, not just a stronger hover. Keyboard focus should be visible, and the same action should work with a pointer or keyboard.

Check narrow, medium, and wide layouts. Check light and dark mode together. A decision that works only in a component specimen is not finished.

## What the guards are for

The automated checks protect decisions that are easy to accidentally undo: readable contrast, the shared type scale, and the color system’s light/dark structure.

They are guardrails, not judges. Each one has a named exception list with room to explain why a specific design needs to differ. Adding an exception is normal when it reflects a deliberate design choice. If the exceptions start pointing in one direction, improve or remove the rule instead of accumulating workarounds.

A passing check means the system is internally consistent. Looking at the actual interface tells us whether the system is good. We need both.

## A good change leaves a trace

When you change the system, make the decision easy to find:

- update the relevant token, component, or guidance;
- show the result on the design-system site where useful;
- use the new thing in real product work, not only in documentation;
- note the reason in plain language close to the decision.

The goal is not to freeze Buzz into a style. It is to give every future designer and builder a clear starting point—and the confidence to improve it when the work asks for more.
