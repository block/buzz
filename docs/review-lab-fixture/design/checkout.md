# Design fixture: remove a saved payment method

Deliberate defect for review testing; this is a written proposal, not application code.

## Requirements

A keyboard or screen-reader user must be able to discover the action and identify
which saved payment method will be removed. Removal requires confirmation.

## Proposed design

Render a 16-pixel trash icon as an unlabelled clickable span next to each method.
Show its tooltip only on mouse hover. Clicking the icon removes the method immediately.

## Expected review finding

The proposal has no keyboard focus or accessible name and omits confirmation.
Recommend a labelled button identifying the method and a confirmation step with
predictable focus return. Verify keyboard and screen-reader operation.
