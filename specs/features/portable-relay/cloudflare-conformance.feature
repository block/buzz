# id: portable-relay-cloudflare-conformance
# type: feature
# story: specs/stories/portable-relay/prove-cloudflare-portability.md
# journey: specs/journeys/promote-portable-relay-to-cloudflare.md
# model: specs/models/portable-relay/portable-relay-cloudflare.model.yaml
# contracts: specs/contracts/openapi/local-relay.yaml, specs/contracts/asyncapi/local-relay.yaml

@portable-relay @cloudflare @conformance
Feature: Portable relay Cloudflare adapter conformance
  As a local-first builder
  I want one stable relay node to preserve portable behavior on Cloudflare
  So that shared memory can outlive my laptop without changing its meaning

  Background:
    Given an adapter declares "portable-relay-cloudflare-v0.1"
    And it declares "portable-relay-core-v0.1"
    And the adapter has an empty SQLite-backed Durable Object
    And the Worker routes the fixture stable node key to that object

  @routing @happy-path
  Scenario: Route one stable node key to one durable boundary
    Given equivalent request authorities normalize to the fixture stable node key
    When each request submits or queries through a new Worker isolate
    Then every request observes the same relay history
    And no request-local or module-global state is required for continuity

  @routing @isolation
  Scenario: Isolate separate stable relay nodes
    Given two different normalized stable node keys
    And the first node contains the portable signed-event vector
    When I query the second node by the known event ID
    Then the second node returns no event
    And its count remains zero
    And neither node's operation mutates the other node

  @core @protocol
  Scenario: Preserve the portable signed event through Cloudflare
    Given the portable signed-event conformance vector
    When I submit it through POST /events for the fixture node
    Then the response is the portable accepted decision
    And POST /query returns the exact signed event envelope
    And POST /count returns one
    And a WebSocket REQ returns the same event before EOSE

  @health
  Scenario: Report preview readiness without implying conformance
    Given the preview Worker can route a request to the relay adapter
    When a client requests GET /health
    Then the response reports ready
    But readiness alone does not prove portable relay conformance

  @durability @output-gate
  Scenario: Make durable history recoverable before acknowledgement
    Given a durable event submission is in progress
    When the journal or projection storage transaction fails
    Then the adapter does not return an accepted acknowledgement
    And the event is absent from durable history and live delivery
    But when the transaction commits successfully
    Then the accepted acknowledgement may be observed

  @durability @eviction
  Scenario: Recover effective state after Durable Object eviction
    Given a valid durable event has been acknowledged
    When the Durable Object instance is evicted without deleting storage
    And a later request reconstructs the object
    Then querying by event ID returns the exact event
    And the effective count is one
    And a duplicate submission creates no second journal record

  @durability @deployment
  Scenario: Preserve history across a compatible code deployment
    Given preview durable history contains the portable signed-event vector
    When a compatible adapter revision replaces the running Worker
    Then the stable node key resolves to the existing Durable Object state
    And querying returns the exact event
    And the conformance evidence records the new adapter revision

  @websocket @hibernation
  Scenario: Preserve subscription meaning across hibernation
    Given a WebSocket subscription has received matching history and EOSE
    And its filters are stored in SQLite
    And its serialized attachment contains bounded connection references
    When the Durable Object is evicted with WebSockets configured to hibernate
    And a later matching event is accepted
    Then the resumed subscription receives the exact live event
    And a non-matching subscription receives nothing
    And CLOSE stops later delivery

  @ephemeral @eviction
  Scenario: Keep ephemeral events out of Cloudflare durable state
    Given a live subscription matches a valid ephemeral event
    When the event is accepted
    Then the subscription receives the exact event
    And SQLite durable history remains unchanged
    When the Durable Object is evicted and reconstructed
    Then the ephemeral event is absent from query and count

  @unsupported
  Scenario: Fail explicitly for an undeclared portable extension
    Given the Cloudflare adapter does not declare NIP-50 search
    When a client submits a NIP-50 search filter
    Then the operation returns an explicit unsupported-capability result
    And it does not return an unfiltered event set

  @identity @profile-gate
  Scenario: Do not imply identity conformance from Cloudflare deployment
    Given the adapter has not declared "portable-relay-identity-v0.1"
    When a client relies only on a Cloudflare route, Access header, or object name
    Then the adapter does not treat that value as a portable authenticated principal
    And its capability evidence reports identity as unsupported

  @identity @replay
  Scenario: Preserve consumed proof state when identity is declared
    Given the adapter also declares "portable-relay-identity-v0.1"
    And fresh audience-bound evidence has authenticated once
    And the evidence declares a durable-object-scoped replay-state lifetime
    When the Durable Object is evicted
    And the same proof is presented again
    Then authentication is denied with "replay_detected"
    And no authentication event or replay marker appears in event history

  @evidence @local-runtime
  Scenario: Distinguish local runtime conformance from deployed conformance
    Given all scenarios pass inside the local Workers test runtime
    When no deployed preview has been exercised
    Then the adapter status is "local-runtime-conformant"
    And it is not reported as "implemented"

  @evidence @deployed-preview
  Scenario: Compare a deployed preview with the laptop adapter
    Given local Workers-runtime conformance passes
    And a non-production preview node is deployed
    When the black-box runner executes the shared vectors against both adapters
    Then event IDs, signed envelopes, decisions, queries, counts, and ordering agree
    And eviction or deployment recovery returns the accepted durable history
    And the adapter may be reported as implementing "portable-relay-cloudflare-v0.1"
