# Agent management provenance

The shared **Not managed on this device** cloud marker means a viewer-owned
identity has no exact-key record in successfully loaded local managed inventory.
The glyph is the approved visual convention, not proof of physical cloud hosting.
Locally managed provider-backed agents, unknown/failed local inventory, unknown
owners, and other people's identities do not get the marker. Neither lifecycle
state nor relay presence decides management provenance.

`KnownAgentPubkeysProvider` supplies stable inventory and ownership context to
identity surfaces; rows must not create their own directory query observers.
Picker candidates preserve exact local-management flags and use the same pure
predicate. Profile ownership can supplement the shared directory evidence.

Pickers, member cards, authors, markdown mention chips, address controls, hover
cards, profile headers/hero/subviews, new-message recipients and DM headers/sidebar
rows use the same glyph and accessible label. The marker grants no membership,
mention eligibility, availability or local management capability. This change does
not change profile navigation, presence, native discovery or invitation/routing.

Regression gates: `otherSetupAgent.test.mjs`, `useKnownAgentPubkeys.test.mjs`,
`buildMentionCandidates.test.mjs`, `MentionAutocomplete.test.mjs`, and
`tests/e2e/cloud-provenance.spec.ts`. Browser evidence is mock-Tauri rendered UI,
using an already eligible channel member and an existing DM, not a discovery or
invitation acceptance test. `mentions.spec.ts` retains its marker-label assertions.
The cloud smoke waits (at most 10 seconds, with per-query status diagnostics) for
successful identity, local inventory and relay directory reads before opening the
picker. Both immediate and delayed-directory fixtures exercise the same rendered
workflow. Each cloud assertion retains its own normal deadline: successful data
loading must not hide a missing glyph or accessible marker.
