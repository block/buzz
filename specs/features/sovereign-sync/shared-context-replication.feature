# id: sovereign-shared-context-replication
# type: feature
# story: specs/stories/sovereign-sync/replicate-shared-context-events.md
# journey: specs/journeys/replicate-shared-context-through-rendezvous.md
# model: specs/models/sovereign-sync/sovereign-stream-agreement.model.yaml
# contract: POST /replication, POST /replication/read, POST /query

@sovereign-sync @shared-context @replication
Feature: Replicate a shared context through rendezvous custody
  As a sovereign node operator
  I want selected context events available through a rendezvous
  So that participants retain independent authority and durable local history

  Background:
    Given two sovereign participants share NIP-29 context "clinical-review"
    And the replication stream ID is "shared/clinical-review-v1"
    And the stream selection includes the context h tag and required metadata
    And the required agreement and transport grants are current

  @identity
  Scenario: Keep context and stream identifiers distinct
    When the source exports the shared context
    Then "clinical-review" identifies the human-visible NIP-29 space
    And "shared/clinical-review-v1" identifies the event delivery selection
    And neither identifier is used as the other's authority claim

  @custody @happy-path
  Scenario: Custody exact context events at the rendezvous
    Given the source journal contains signed events in and outside the shared context
    When the authenticated source replicates the configured stream
    Then the rendezvous accepts exact signed envelopes for selected context events
    And unrelated events are absent from that export
    And the rendezvous does not become an event author or policy owner

  @resume @checkpoint
  Scenario: Advance the destination cursor only after safe ingest
    Given the destination reads an ordered page after its durable cursor
    When every record receives a checkpoint-safe destination receipt
    Then the destination persists the returned opaque cursor
    And a retry produces no duplicate logical event

  @resume @failure
  Scenario: Preserve the previous cursor after an unsafe outcome
    Given the destination reads an ordered page after its durable cursor
    When any record lacks a checkpoint-safe destination receipt
    Then the destination does not persist the returned cursor
    And the page remains eligible for retry

  @client
  Scenario: Browse replicated history as an ordinary context
    Given the destination has accepted the selected signed events
    When a client queries the shared context h tag
    Then it receives the exact effective context events
    And the client need not understand sync declarations or rendezvous topology
