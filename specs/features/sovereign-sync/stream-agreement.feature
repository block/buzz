# id: sovereign-stream-agreement
# type: feature
# stories: specs/stories/sovereign-sync/offer-sovereign-event-stream.md, specs/stories/sovereign-sync/accept-sovereign-event-stream.md, specs/stories/sovereign-sync/authorize-sovereign-stream-transport.md, specs/stories/sovereign-sync/reconcile-stream-agreement-drift.md
# journey: specs/journeys/offer-and-accept-sovereign-event-stream.md
# model: specs/models/sovereign-sync/sovereign-stream-agreement.model.yaml
# contract: POST /events, POST /query

@sovereign-sync @agreement
Feature: Establish a sovereign event-stream agreement
  As a sovereign node operator
  I want owner consent and transport authorization to remain distinct
  So that independently owned nodes share only an explicitly reviewed event stream

  Background:
    Given source owner A and destination owner B have distinct application identities
    And each node has a stable principal and independently rotating transport keys
    And stream "shared/clinical-review-v1" selects one immutable NIP-01 filter set

  @offer @happy-path
  Scenario: Offer an immutable event selection to a destination owner
    When source owner A publishes the current export declaration
    Then the declaration is authored by source owner A
    And its stream ID is "shared/clinical-review-v1"
    And its p tags include destination owner B
    And its content contains the explicit event selection
    But the offer grants no destination ingest or stream-read authority

  @accept @happy-path
  Scenario: Accept the exact export head with separate transport keys
    Given source owner A has offered the current export to destination owner B
    When destination owner B publishes an active admit for the same stream
    Then the admit e tag equals the current export event ID
    And the admit p tags name transport keys accepted by the destination sink
    And the export and admit form a matched agreement

  @identity @separation
  Scenario: Do not substitute a transport key for the destination owner
    Given the export offers to destination owner B
    And an admit is authored by destination transport key B1 instead
    When agreement state is evaluated
    Then the agreement is unmatched because B1 is not destination owner B
    But B1 may still authenticate transport if an owner-authored grant names it

  @pinning @validation
  Scenario Outline: Refuse to match a defective admit
    Given source owner A has an active current export offered to destination owner B
    And destination owner B has an active admit with <defect>
    When agreement state is evaluated
    Then the state is "<state>"
    And no transport outcome changes that state

    Examples:
      | defect                                  | state     |
      | no export pin                           | unpinned  |
      | a pin to the previous export head       | drifted   |
      | a different stream ID                   | unmatched |
      | a revoked status                        | inactive  |

  @transport @orthogonal
  Scenario: Report agreement and transport readiness independently
    Given the current export and admit form a matched agreement
    But a pull topology has no active read grant for destination transport key B1
    When relationship state is evaluated
    Then agreement state is "matched"
    And transport state is "partially_authorized"

  @selection @new-stream
  Scenario: Change event selection under a new stream ID
    Given "shared/clinical-review-v1" has a matched agreement
    When source owner A changes the event selection
    Then the replacement offer uses a new stream ID
    And the prior agreement becomes superseded
    And destination owner B must review and pin the new export independently

  @revocation @no-fallback
  Scenario: Keep a revoked journal-governed domain empty
    Given destination admission is governed by owner-signed declaration heads
    When destination owner B revokes its final active admit
    Then the destination admits no transport key for that stream
    And stale file or environment trust does not reactivate admission
