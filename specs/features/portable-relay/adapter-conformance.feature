# id: portable-relay-adapter-conformance
# type: feature
# story: specs/stories/local-relay/run-without-hosted-infrastructure.md
# journey: specs/journeys/start-durable-local-buzz.md
# model: specs/models/portable-relay/portable-relay-boundary.model.yaml
# contracts: specs/contracts/openapi/local-relay.yaml, specs/contracts/asyncapi/local-relay.yaml

@portable-relay @conformance
Feature: Portable relay adapter conformance
  As a local-first builder
  I want relay adapters to preserve the same signed-event behavior
  So that an experiment can move between laptop and cloud runtimes unchanged

  Background:
    Given an adapter declares the "portable-relay-core-v0.1" profile
    And the adapter starts with an empty durable journal

  @identity @happy-path
  Scenario: Preserve a signed event across adapters
    Given the portable signed-event conformance vector
    When I submit its event through the HTTP event operation
    Then the decision is accepted for the fixture event ID
    And querying by that event ID returns the exact signed event envelope
    And the adapter has not rewritten its author, kind, tags, content, or signature

  @durability
  Scenario: Recover an acknowledged durable event
    Given a valid durable event has been accepted
    When the adapter runtime is stopped and reopened over the same journal
    Then querying by the event ID returns the event
    And the effective event count is one

  @validation
  Scenario: Reject a tampered event without effects
    Given a signed event whose content no longer matches its ID and signature
    When I submit the event
    Then the decision is rejected
    And the durable journal is unchanged
    And no live subscription receives the event

  @idempotency
  Scenario: Treat duplicate submission consistently
    Given a valid durable event has been accepted
    When I submit the exact event again
    Then the decision is accepted as a duplicate
    And the effective event count remains one
    And the durable journal contains one record for the event ID

  @replacement
  Scenario: Rebuild the same effective replacement state
    Given multiple valid replacement events share one replacement key
    When the adapter reduces and replays their accepted history
    Then both executions select the event with the greatest created_at
    And an event-ID tie selects the lexicographically smaller event ID

  @ephemeral
  Scenario: Keep ephemeral events live-only
    Given a live subscription matching a valid ephemeral event
    When I submit the ephemeral event
    Then the live subscription receives the exact signed event
    And the event is absent from durable history
    And the event is absent after adapter restart

  @websocket
  Scenario: Complete historical delivery before continuing live
    Given durable history contains an event matching my NIP-01 filters
    When I open a WebSocket REQ subscription
    Then I receive the matching historical EVENT
    And I receive EOSE for the subscription
    And later matching accepted events are delivered until I send CLOSE

  @transport
  Scenario: Share one state boundary across HTTP and WebSocket
    Given I submit a valid durable event through POST /events
    When I query it through a WebSocket REQ
    Then I receive the exact submitted event
    When I submit another valid durable event through WebSocket EVENT
    Then POST /query returns the exact second event

  @unsupported
  Scenario: Fail explicitly for an unsupported extension
    Given the adapter does not declare the NIP-50 search capability
    When I query with a NIP-50 search filter
    Then the query fails with an explicit unsupported-capability result
    And the adapter does not return an unfiltered event set

  @policy
  Scenario: Keep policy outside the event kernel
    Given an adapter also declares the "portable-relay-policy-v0.1" profile
    And its policy denies the authenticated actor
    When the actor submits an otherwise valid signed event
    Then the decision is rejected before journal mutation
    And no live subscription receives the event

