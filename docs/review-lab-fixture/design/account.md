# Design fixture: account validation feedback

Deliberate defect for review testing; this is a written proposal, not application code.

## Requirements

When saving fails validation, users must understand which field needs correction
and how to fix it, including without color perception or visual access.

## Proposed design

Turn the invalid input's border from gray to red. Do not show an error message,
change focus, or announce the failure. Keep the Save button and its label unchanged.

## Expected review finding

Color alone neither explains the error nor communicates it to a screen reader.
Provide actionable text associated with the field, expose its invalid state, and
announce the failed save. Verify the error is understandable without color.
