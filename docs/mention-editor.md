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

## Exact recipient labels

A selected label is a binding to one exact public key, not a lookup by the latest
profile name. Selecting a second identity with the same name reserves a qualified
label containing its full key (and, if needed, a collision suffix). Team members
reserve labels sequentially. Automatic addressing inserts/restores/removes that
registered label, never a different recipient with the same name.

Manually typed member names with multiple exact-key matches are rejected with a
visible instruction to use the mention picker. Chat, edits and standalone forum
composition retain their draft and publish nothing on this error. An edit may
remove an ambiguous historical label; when the old content cannot be resolved,
all recipients in the valid replacement are revalidated. Selection stays bound
across profile renames. This does not expand eligibility or change relay
revalidation, invitation or publication authorization.

Coverage: `useMentions.test.mjs`, `useAgentAddressLockPicker.test.mjs`,
`submitMessageEdit.test.mjs`, `mention-recipients.spec.ts`, and the existing
same-name agent case in `mentions.spec.ts`. The integration-project
`onboarding.spec.ts` checks that an ambiguous Fizz mention cannot complete the
welcome flow, then selects the exact newly started starter and asserts its sole
recipient tag before checking the original completion and layout behavior.

## Exact occurrences and edit history

Draft presence, extraction, audience removal/restoration and persona preparation
share longest literal occurrence ownership, including typed-member and persona
competitors. A shorter alias cannot claim another label's qualified key or
collision suffix. Removing one recipient must preserve a different recipient's
full label and exclude only the removed identity from the composed send audience.

Rendering and edit hydration reconstruct qualified labels only for identities
already present in event `p` or `mention` tags. Body text alone does not authorize
a key. Tag order cannot resolve ambiguous aliases; an unqualified label is
restored only when one candidate remains. Qualified tagged labels can survive
profile renames or missing profiles. Historical arbitrary labels are not stored:
missing or genuinely ambiguous unqualified aliases remain unresolved,
non-notifying references rather than guessed recipients. Edit regression coverage
checks reference preservation, not a claim of new notifying edit `p` tags.

Occurrence recognition must retain every competing literal label before binding
eligibility is applied. The historical resolver's `mentionNames` includes
ambiguous aliases as blockers; its explicit `mentionPubkeysByName` map binds only
resolved aliases. Renderers leave unbound occurrences as literal text. Body-based
ordering and legacy send-to-channel fallback use both outputs, not the map alone.
Edit-open fallback refs enter the same candidate composition as current selected
refs, typed members and personas before presence selection; they are never matched
in a separate, narrower pass. Current selections (including unbound personas)
take precedence over same-label fallback refs. Historical unresolved identities
still preserve non-notifying metadata, without claiming a literal binding.

## Replacement edit snapshots

An edit snapshot supersedes original notification `p` tags for authored-body
identity, including on the next edit-open. Unresolved historical aliases are
retained only if their original literal still wins current draft occurrence
ownership; a newly selected same-label key or a longer typed label cannot inherit
those old references. With no known historical alias, retain metadata only for
an unchanged body with unchanged bindings (conservatively drop it on replacement).
Current picker bindings must never reinterpret the original recipient audience.
All historical aliases compete together, including unresolved aliases absent from
the current roster and labels containing nested `@` signs. Tied historical keys
remain reference candidates, never a name-to-key binding. The same non-binding
historical labels also compete during resolved-ref snapshots and edit recipient
extraction, so a restored shorter binding cannot reclaim a deleted occurrence.
Current same-label members remain eligible; only an explicit current selection
supersedes same-label historical references.
New typed recipients join the reference snapshot after eligibility revalidation.

Send-to-channel uses the latest authored snapshot rather than intersecting it
with the original event's `p` tags: a recipient added on an earlier edit may have
no `p` on the original event. This is a new forward, not an edit notification.
Immutable annotated automatic-address metadata remains separate, and only
previously delivered automatic addresses are forwarded. Snapshot bodies and
full-key qualifiers alone never authorize an untagged recipient.

Full-key literal labels remain intact in the composer and on the wire. Readonly
mention chips abbreviate only their bound public key with the shared
`truncatePubkey` display form (eight leading characters, ellipsis, four trailing).
The complete literal label and exact key remain in metadata, title and profile
target; whole-chip copy restores the full label for paste/edit round trips.
Abbreviations are recognition aids, never recipient lookup keys. Partial copies
remain plain text. Composer and rendered chips still wrap within narrow lines.
The browser regression covers 800px windows at 100% and 150% root text size,
send/reopen, and historical replacement followed by forwarding.

Ordinary composer mentions retain their human, agent, or channel identity icon.
Full-key literal decorations explicitly opt into wrapping with a visible text
prefix and no cloned inline padding or pseudo-icon; `spellcheck=false` is not a
presentation marker. Readonly mention chips keep their own wrapping/accessibility
contract. Menu-based edit activation waits for Radix exit-focus cleanup before
loading/focusing the editor; navigation tests must observe edit content and focus,
not treat an already enabled reply input as an activated edit.
