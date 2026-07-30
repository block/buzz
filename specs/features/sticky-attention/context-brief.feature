# id: sticky-attention-context-brief
# type: feature
# stories: specs/stories/sticky-attention/re-enter-with-a-context-brief.md, specs/stories/sticky-attention/accrete-attention-residue.md, specs/stories/sticky-attention/distill-a-context-head.md, specs/stories/sticky-attention/attribute-context-contributions.md
# journey: specs/journeys/re-enter-a-bounded-context.md
# model: specs/models/sticky-attention/bounded-attention-context.model.yaml
# contract: POST /events, POST /query

@sticky-attention @brief
Feature: Re-enter a bounded context through a projected brief
  As a sovereign builder
  I want orientation, residue, and distillation to be properties of one
  h-scoped event set on my own node
  So that any surface can reassemble the room in under a minute and every
  claim in it is attributable

  Background:
    Given builder B owns the node and binds agents "claude-code" and "codex" with current owner attestations
    And a mandated steward key and an admitted counterpart identity also write into context "portable-relay"
    And context "portable-relay" is bounded by exactly one h identifier
    And all projections read only the local sovereign relay

  @re-entry @happy-path
  Scenario: Project an orientation, not a transcript
    Given context "portable-relay" has heads, residue, and one claimed handoff
    When any surface projects the brief
    Then the brief renders current understanding from the context's head cells
    And a live thread of recent residue in causal order
    And the claimed handoff as an open loop
    And changes since the attention cursor with foreign contributions first
    And raw history is one step away but is not the default rendering

  @cursor @derivation
  Scenario: Derive the attention cursor from the builder's own agents only
    Given the newest events in the context are, in order: a counterpart note, a steward report, and an older "codex" session record
    When the cursor is derived
    Then the cursor is the "codex" session record
    And the counterpart note and steward report appear in changes since the cursor

  @determinism @surfaces
  Scenario: Identical brief on every surface
    Given the CLI, the desktop app, and a second agent session project the same context
    When each projection folds the same event set
    Then all three briefs are identical
    And no projection stored the brief as authority

  @provenance @attribution
  Scenario: Attribute every element by signature class
    Given the brief contains a head proposed by "claude-code", residue from "codex", a steward finding, and a counterpart note
    When provenance is rendered
    Then each element names its signing identity
    And elements signed with a current owner attestation render as verified
    And the builder's agents, the steward, and the counterpart render as distinct contributor classes

  @provenance @degraded
  Scenario: Never blend unverified content into verified content
    Given a residue record whose signer's owner attestation has expired
    When the brief is projected
    Then the record is displayed with an unverified marking
    And it is never rendered indistinguishably from verified records
    And no label in its content upgrades its status

  @residue @ceremony-free
  Scenario: Residue accretes as a side effect of working
    Given "claude-code" lands a decision while working in the context
    When it appends a session record carrying the context's h boundary
    Then the record is durable once the local relay accepts it
    And the next brief projection includes it with no further action
    And no step required the builder to name a scope or run a save command

  @residue @boundary
  Scenario: Report unbounded residue instead of counting it
    Given a legacy head exists in a slug namespace without an h boundary
    When the context is projected
    Then the legacy head is excluded from the brief
    And it is reported as unbounded state awaiting migration

  @distill @happy-path
  Scenario: Distill the head from accumulated residue
    Given residue has accreted since the previous head and the context reads as accreting
    When "claude-code" proposes a distillation and builder B accepts it
    Then the head cell is replaced within the same h boundary
    And the head records state, open threads, and rationale
    And the head names the proposing agent
    And all prior residue remains replayable
    And the context reads as fresh

  @distill @staleness
  Scenario: Surface staleness at re-entry
    Given many residue records and several distinct efforts have accreted under one head
    When the brief is projected
    Then the context reads as stale
    And the brief prompts distillation or a boundary split
    And staleness is never suppressed

  @warmth @listing
  Scenario: List contexts by warmth for recognition, not recall
    Given contexts with attention cursors of one hour, two days, and three weeks ago
    When the surfaces list bounded contexts
    Then contexts are ordered newest cursor first
    And each shows title, last-touch time, and touching identity
    And a counterpart's recent write does not raise a context above the builder's own touches

  @v2 @planned @navigation
  Scenario: Enter a context by question
    Given the builder asks "what do we know about the steward's mandate"
    When navigation by question is available
    Then the surface resolves the question to a bounded context and position
    And the brief opens there

  @v2 @planned @relationships
  Scenario: Map a bounded context across nodes
    Given the same bounded context is inhabited from a second sovereign node
    When context mapping between nodes is specified
    Then cross-node continuity composes with existing stream agreements
    And provenance classes extend to the second node's identities
