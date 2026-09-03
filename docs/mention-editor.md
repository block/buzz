# Mention editor contract

Autocomplete inserts a full literal label followed by a separator. Full labels
may contain spaces; internal spaces must not be mistaken for the final boundary.
The immediate next typed character goes after the separator even if the browser
remaps the DOM caret to the highlight edge. Explicit ArrowLeft or click cancels
that settlement so users can intentionally edit a mention. Plain typed tokens
retain their existing behavior; this does not change recipient resolution.

Regression coverage: `mentionHighlightExtension.test.mjs` simulates chip-edge and
whitespace-run rewrites, internal spaces, and deliberate motion. The ordinary
member browser cases in `mention-spacing.spec.ts` test immediate typing and
ArrowLeft without requiring remote discovery or invitation; `mentions.spec.ts`
also covers clicking chip edges and insertion before existing text.
