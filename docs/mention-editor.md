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

## Stable completion choices

Changing completion text requests a new list. The picker shows loading while
needed data arrives, then installs one set of up to 50 choices. Background
membership, directory, presence and ranking updates do not replace or reorder
those choices. A subsequent text change or explicit open uses current evidence.
Create/Add membership refreshes discovery for that next open, not a moving list.

Arrow keys select an index in the displayed set. Tab, plain Enter and clicking
choose that displayed identity, including same-name rows; they do not require
global uniqueness or exhaust search pagination. Space is still implicit exact
name completion: partial, longer-name and ambiguous matches stay literal.

Leaving the completion, dismissal and navigation abandon its request and
selection. Closed or superseded requests cannot install their results. Explicit
no-trigger menus open/reset normally; toggling an automatic address reopens a
fresh menu rather than preserving selection across an edited document.

This is display stability, not cached permission. Selection checks current
exact-key access and team recipients; publication still revalidates authority.
Recipient-label binding and highlight settlement remain independent of the
picker's request lifecycle.

Availability labels may resolve from Checking to Mention or Unavailable in place;
this never replaces an identity, label, order or selected index. Retry starts a
fresh evidence lookup, not a new chooser request: installed rows (including an
installed empty/error result) are not replaced or reordered. If initial results
have not yet installed, they still wait for the required discovery to settle.
Change the completion text or explicitly reopen to discover a new set of choices.
Live access is checked again at selection, including for rows whose display
snapshot originally permitted mentioning.
Failed or pending Retry does not authorize cached choices.

## Fresh selection and pin admission

Chat pointer, Enter, Tab and exact-Space choices, explicit automatic-address
adds (including the closed-picker default-agent shortcut), and standalone forum
choices share one cancellable prepare/commit operation. Discovery and cached
availability are preflight only: the exact selected agent keys and destination
are freshly revalidated in prepare mode. A team is admitted as its captured
recipient set, never partially or by resolving its name again. Human-only and
unresolved-persona choices retain their existing recipient semantics.

No label reservation, selected-agent intent, successful-selection history,
editor replacement, automatic audience or generated-prefix provenance is added
while checking. A single fenced synchronous commit owns those effects. The raw
insertion implementation is private; restoration/registration of existing draft
intent is not a new user selection. The displayed-row WeakMap associates live
availability overlays with installed row identities; it is not a retained-list
or moving-ranking authority. The closed-picker shortcut holds at most one exact
issued choice, bound to the editor/query revision.

“Checking access…” is visible and announced in chat and forum. Access denial
leaves the draft unchanged and says access changed; failed lookup or the
15-second timeout says access could not be checked. Selecting again retries
with fresh evidence. A duplicate action for the same pending choice is consumed,
not queued for publication. A different choice supersedes it. Typing, native
input, selection transactions, dismissal, navigation, submission and unmount
abandon pending work; returning to the same text, caret or scope does not revive
it. Native editing is never disabled by selection admission. Unexpected commit
errors are reported as a failure to finish selection (inspect the draft before
retry), not mislabeled as an authority denial.

Unpin/removal remains available without permission and cancels pending adds.
Existing automatic-address restoration remains separate from new pin admission.
Native Enter outside the chooser and non-exact/ambiguous Space retain their
normal handling. Only successful insertion settles the mention caret.

Selection is not a send permit: publication independently revalidates authority
for the actual destination, including revocation after successful selection.
Offline fresh admission therefore fails closed without clearing the draft.
Loss of live choice eligibility abandons a pending admission immediately; a
later Retry restoring that same row cannot revive the older operation.
Focused coverage in `freshMentionChatJourney.test.mjs` mounts the production
chat picker with Tiptap and the actual standalone `ForumComposer`; its DOM is
JSDOM with fixture IPC, **not** browser or live-relay evidence. Browser,
relay-backed revocation, full-package gates and independent review remain
separate release gates.
