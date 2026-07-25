# id: portable-relay-identity-conformance
# type: feature
# story: specs/stories/portable-relay/control-attributable-access.md
# journey: specs/journeys/start-durable-local-buzz.md
# model: specs/models/portable-relay/portable-relay-identity.model.yaml
# contract: specs/architecture/portable-relay-identity-v0.1.md application ports

@portable-relay @identity @conformance
Feature: Portable relay cryptographic identity
  As a local-first builder
  I want caller identity to remain distinct from signed-event authorship
  So that people, agents, and relay nodes receive only explicitly granted access

  Background:
    Given an adapter declares the "portable-relay-identity-v0.1" profile
    And the adapter starts with an empty durable journal
    And protected operations deny anonymous callers unless policy declares them public

  @authentication @happy-path
  Scenario: Bind fresh challenge evidence to its signing principal
    Given fresh audience-bound NIP-42 evidence signed by the identity vector author
    When the adapter authenticates the WebSocket connection
    Then the authenticated principal is the evidence signing public key
    And the authentication event is absent from durable history

  @authentication @validation
  Scenario Outline: Reject evidence that is not valid for this security context
    Given otherwise well-formed authentication evidence with <defect>
    When the adapter authenticates the protected operation
    Then authentication is denied with "<code>"
    And the durable journal is unchanged
    And no reusable authentication material is logged as an event

    Examples:
      | defect                     | code              |
      | an invalid signature       | invalid_evidence  |
      | another relay audience     | audience_mismatch |
      | an expired timestamp       | evidence_expired  |
      | an already consumed nonce  | replay_detected   |

  @append @happy-path
  Scenario: Admit a direct event from its authenticated author
    Given the authenticated Nostr principal equals event.pubkey
    And destination append policy admits the event and scope
    When the principal directly submits the portable signed-event vector
    Then the event is eligible for normal portable relay ingest
    And the exact signed event envelope is preserved

  @append @authorization
  Scenario: Deny a direct event signed by another principal
    Given the authenticated Nostr principal does not equal event.pubkey
    And no supported verified delegation covers the event
    And no declared privacy-envelope rule covers the event kind
    When the principal directly submits the otherwise valid event
    Then append is denied with "author_mismatch"
    And the durable journal is unchanged
    And no live subscription receives the event

  @append @delegation
  Scenario: Honor only a verified and scoped delegation
    Given the adapter declares support for a delegation mechanism
    And a controller's cryptographic delegation names the agent principal
    And its conditions cover the submitted event and evaluation time
    When the authenticated agent directly submits its signed event
    Then the delegated principal is eligible for destination policy
    But an expired, malformed, self-asserted, or out-of-scope delegation is denied with "delegation_invalid"

  @append @privacy
  Scenario: Limit privacy-envelope author indirection to declared event kinds
    Given the adapter declares a NIP-59 gift-wrap author-indirection rule
    And the authenticated principal submits a structurally valid gift wrap
    When event.pubkey intentionally differs from the authenticated principal
    Then the event is eligible for destination policy under that narrow rule
    But an ordinary mismatched-author event remains denied with "author_mismatch"

  @replication @peer-binding
  Scenario: Bind a replication source to an authenticated stable node
    Given destination trust binds "laptop-a/coherence" to "did:example:buzz-relay-a"
    And the peer proves an active verification method for that node and destination
    When the peer presents a record from "laptop-a/coherence"
    Then the peer binding succeeds
    And the transported event retains its original event.pubkey
    And destination event verification and policy still run

  @replication @authorization
  Scenario Outline: Reject replication identity substitution
    Given destination trust binds the configured source to the identity vector relay node
    When replication arrives with <substitution>
    Then peer authentication is denied with "<code>"
    And the replication sink is not invoked
    And the destination journal is unchanged

    Examples:
      | substitution                              | code            |
      | only a self-asserted source ID             | peer_unbound    |
      | a valid proof from a different relay node  | source_mismatch |
      | a proof from a revoked node key            | invalid_evidence |

  @read @authorization
  Scenario: Apply the same disclosure rule to every read surface
    Given a protected event exists for one authorized reader
    And another authenticated principal requests it
    When that principal queries, counts, subscribes, or awaits live delivery
    Then the protected event is not returned, counted, or delivered
    And a direct lookup by its known event ID does not bypass authorization

  @read @filter-policy
  Scenario: Deny a prohibited filter before querying candidates
    Given destination policy prohibits the reader from requesting the filter scope
    When the reader opens the query
    Then read is denied with "scope_denied"
    And no candidate event content or existence is disclosed

  @confidentiality
  Scenario: Preserve encrypted content without treating authorization as encryption
    Given an authorized reader requests a recipient-encrypted signed event
    When the relay delivers the event
    Then it returns the exact signed ciphertext envelope
    And the relay does not claim that access policy hides plaintext from its operator

  @node-identity @key-rotation
  Scenario: Rotate a node verification key without changing stable identity
    Given "did:example:buzz-relay-a" authorizes a new active verification method
    And its previous verification method is revoked
    When the node authenticates with each method
    Then the active method binds to the existing relay node principal
    And the revoked method is denied with "invalid_evidence"
    And historical event author signatures remain unchanged
