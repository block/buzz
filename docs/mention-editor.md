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

## Pasted mention identities

A pasted mention's `label → pubkey` pair is bound only after this community's
own state vouches for it, and for a non-member that check crosses the network —
so the answer lands after the text is already on screen. Three fences decide
whether that late answer may still write the mention map (`mentionPasteBinding`):

- **Occurrence.** The label must still be in the text *that paste* inserted.
  `PastedMentionOccurrencesExtension` holds one range per in-flight paste and
  maps it through every transaction; the range dies when its content is deleted
  or replaced, and a settlement with no live range binds nothing. The extension
  is registered in the shared composer extension list, so a new composer gets
  the fence by construction — dropping it there costs pasted identities silently.
- **Generation.** The label's newest claim must still be this paste's. Every
  explicit act on a label (picker selection, resolved insert, send-time persona)
  claims it, which retires any paste still verifying that name.
- **Trust.** Unchanged: `useVerifyMentionIdentities` answers from local trusted
  state, then the relay's own profile.

Sends await `settlePendingMentionBindings()` before extracting recipients,
bounded at `PENDING_MENTION_BINDING_TIMEOUT_MS`, so a still-deciding paste
cannot publish a readable `@Label` with no `p` tag.

Regression coverage: `pastedMentionOccurrences.test.mjs` (range ownership
through the real plugin), `mentionPasteBinding.test.mjs` (ordering, through the
production hook), and the send-window, namesake, and delete-then-retype cases in
`mention-clipboard.spec.ts`.
