Feature: Durable local Buzz event log
  As a local-first builder
  I want a single-process relay with portable storage
  So that a person and accountable agents can build shared memory on a laptop

  Background:
    Given no Postgres, Redis, MinIO, or Docker service is running
    And the local relay is configured with a temporary event log

  Scenario: Accept and query a signed event
    Given a valid signed kind 1 event
    When I submit the event to POST /events
    Then the response is accepted
    And POST /query by its event ID returns the event

  Scenario: Reject a tampered event
    Given an event whose content no longer matches its signed ID
    When I submit the event to POST /events
    Then the response is rejected
    And the event log is unchanged

  Scenario: Recover durable history after restart
    Given the relay has accepted a durable signed event
    When I stop and reopen the relay with the same event log
    Then a query by event ID returns the event

  Scenario: Treat a duplicate as idempotent
    Given the relay has accepted a durable signed event
    When I submit the same event again
    Then the response is accepted as a duplicate
    And the event appears only once in queries
    And the event appears only once in the event log

  Scenario: Do not recover ephemeral events
    Given a live subscription for kind 20001
    When I submit a valid signed kind 20001 event
    Then the live subscription receives the event
    And the event is not appended to the event log
    And the event is absent after relay restart

  Scenario: Complete a WebSocket historical subscription
    Given the relay has accepted a durable signed event
    When I send a matching NIP-01 REQ frame
    Then I receive an EVENT frame containing the event
    And I receive an EOSE frame for the subscription
