# id: journal-handoff-lifecycle
# type: feature
# stories: specs/stories/journal-handoff/open-attributable-work-offer.md, specs/stories/journal-handoff/claim-delegated-work-exclusively.md, specs/stories/journal-handoff/return-verifiable-work-evidence.md, specs/stories/journal-handoff/close-verified-handoff.md
# journeys: specs/journeys/delegate-work-through-journal-handoff.md, specs/journeys/return-and-close-delegated-work.md
# model: specs/models/journal-handoff/journal-handoff.model.yaml
# contract: POST /events, POST /query

@journal-handoff @lifecycle
Feature: Delegate, return, and close work through journal handoffs
  As a sovereign node operator
  I want every delegation transition to be a signed, causally ordered event
  So that any party can re-derive who owes what to whom from the journal alone

  Background:
    Given delegating owner O and claimant agent key C have distinct identities
    And C carries a valid NIP-OA owner attestation from its owner
    And both nodes admit the shared context "buzz-evolution"
    And lifecycle state is always reduced from the union of the sovereign and rendezvous views

  @open @happy-path
  Scenario: Open an attributable work offer
    When owner O publishes a handoff open in context "buzz-evolution"
    Then the open carries exactly one lifecycle t tag "handoff:open"
    And exactly one h context "buzz-evolution"
    And a canonical lowercase 40-character base_commit
    And p tags naming the only keys allowed to claim
    And scope, acceptance, and embodiment contracts
    And O is the only owner identity permitted to later close

  @open @validation
  Scenario Outline: Reject a defective open and orphan its chain
    Given an open with <defect>
    When the lifecycle is reduced
    Then the open is invalid with reason "<reason>"
    And every claim, return, and close referencing it is orphaned

    Examples:
      | defect                                | reason                                                          |
      | an abbreviated 12-character commit    | base_commit must be a canonical lowercase 40-character object id |
      | an uppercase-hex commit               | base_commit must be a canonical lowercase 40-character object id |
      | no p tags                             | open event must p-tag at least one allowed claimant             |
      | a target.pubkey absent from p tags    | target.pubkey must also appear in a p tag                       |
      | two h contexts                        | exactly one h tag required                                      |

  @claim @happy-path
  Scenario: Claim with an authorized signature
    Given owner O's open names claimant key C in a p tag
    When C publishes a claim linking the open with an e root tag after the open
    Then the handoff state is "claimed"
    And the accepted claim is C's earliest claim by created_at then id

  @claim @authorization
  Scenario: Ignore label spoofing and unauthorized signers
    Given an open naming only claimant key C
    When key X publishes a claim whose content labels itself runner "C"
    Then X's claim is ignored as an unauthorized signer
    And the content label grants no authority

  @claim @conflict
  Scenario: Park competing authorized claims in deterministic conflict
    Given an open naming claimant keys C1 and C2
    And both C1 and C2 publish valid claims
    When the lifecycle is reduced
    Then the state is "conflicted"
    And no return is accepted until the conflict is resolved
    And reduction of the same events on any node reports the same conflict

  @return @happy-path
  Scenario: Return bound to the exact claim
    Given C holds the accepted claim
    When C publishes a return linking the open with e root and the claim with e claim
    And the return restates the claim_id and carries status "done"
    Then the handoff state is "returned"

  @return @authorization
  Scenario: Refuse a return from the claimant's owner sibling key
    Given C holds the accepted claim
    And C2 is a different key belonging to C's owner
    When C2 publishes a return for the handoff
    Then the return is ignored because the signer is not the exact claimant

  @return @custody
  Scenario: Gate artifact advertisement on rendezvous custody
    Given C's return advertises result artifact digest D as an x tag
    Then before posting, a manifest referencing D exists at the rendezvous in the handoff's h context
    And the bytes are readable through the authenticated artifact path
    And the fetched bytes hash to D

  @return @verification
  Scenario: Verify returned artifacts from an independent node
    Given a posted return advertising artifact digests as x tags
    When an independent node runs handoff verify-artifacts for that return
    Then the return's content artifact list equals its x tags
    And every blob is fetched through the verifier's own reader identity
    And every fetched blob byte-matches its advertised SHA-256

  @close @happy-path
  Scenario: Close over the newest valid return
    Given the newest valid return R from claimant C
    When owner O publishes a close linking the open with e root and R with e return
    And the close restates R's return_id and posts after R
    Then the handoff state is "closed"

  @close @suppression
  Scenario: An earlier-return close cannot suppress a later return
    Given valid returns R1 then R2 from claimant C
    When owner O publishes a close pinning R1
    Then the handoff state is not "closed"
    And the later return R2 remains the newest valid return

  @close @authorization
  Scenario: Only the opener's effective owner may close
    Given a valid return from claimant C
    When a close is published by a signer that is neither owner O nor a signer attested by O
    Then the close is ignored
    And the handoff state remains "returned"

  @close @legacy
  Scenario: A legacy open-only close is not causally valid
    Given a close that links only the open and identifies no return
    When the lifecycle is reduced
    Then the close does not terminate the lifecycle
    And steward findings for the handoff are not suppressed

  @steward @observation
  Scenario: Stewards report invalid opens without mutating them
    Given a mandated steward with observe and report powers
    And an open that fails validation
    When the steward reduces the same event union
    Then the steward reports the open invalid with its reason each cycle
    And the steward publishes no lifecycle transition

  @v0.2 @planned @retirement
  Scenario: Retire a stale open with a journal-verifiable withdrawal
    Given an open that its owner considers stale or mistaken
    When the owner publishes a withdrawal referencing the open
    Then the lifecycle reaches a terminal retired state
    And stewards report the retirement once and then remain quiet

  @v0.2 @planned @exclusivity
  Scenario: Claim exclusively through a relay-enforced primitive
    Given an open naming claimant keys C1 and C2
    When C1 and C2 race an exclusive claim
    Then exactly one claim is accepted at the relay boundary
    And unattended execution becomes safe to enable
