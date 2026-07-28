# id: portable-relay-artifact-conformance
# type: feature
# story: specs/stories/sovereign-sync/retrieve-referenced-stream-artifacts.md
# journey: specs/journeys/fetch-referenced-stream-artifacts.md
# model: specs/models/sovereign-sync/sovereign-stream-agreement.model.yaml
# contracts: specs/contracts/openapi/sovereign-sync.yaml

@portable-relay @artifacts @conformance
Feature: Portable relay artifact conformance
  As a sovereign node operator
  I want artifact adapters and clients to share one custody profile
  So that content-addressed bytes retain the same authority and integrity on every runtime

  Background:
    Given an adapter declares the "portable-relay-artifacts-v0.1" profile
    And portable identity authentication is enabled
    And the portable artifact conformance vector defines the owner, admitted peer, reader, and stranger
    And authenticated artifact requests use fresh payload-bound NIP-98 proofs with nonces
    And accepted signed events may reference artifacts with x tags

  @upload @authorization
  Scenario Outline: Accept uploads only from artifact custodians
    Given <principal> presents a POST proof bound to the exact artifact bytes
    When <principal> uploads the non-empty artifact
    Then the upload is <decision>
    And the bytes are <storage>

    Examples:
      | principal        | decision | storage |
      | the owner        | accepted | stored  |
      | an admitted peer | accepted | stored  |

  @upload @authorization @known-gap-laptop-upload-admission
  Scenario: Deny an authenticated stranger upload
    Given an authenticated stranger is neither the owner nor an admitted replication peer
    When the stranger uploads a valid non-empty artifact
    Then the upload is denied with "scope_denied"
    And no bytes are stored

  @upload @receipt @integrity
  Scenario: Return a content-derived portable receipt
    Given the upload body is the portable artifact fixture
    When the owner uploads the body
    Then the JSON receipt has exactly "sha256", "size", and "url"
    And the receipt sha256 equals the SHA-256 of the upload body
    And the receipt size equals the upload body size
    And the receipt URL addresses the lowercase receipt sha256

  @upload @validation
  Scenario Outline: Reject invalid artifact body sizes
    Given an authenticated owner prepares <body>
    When the owner uploads it
    Then the upload is rejected
    And no artifact bytes are stored

    Examples:
      | body                         |
      | an empty body                |
      | a body larger than 16 MiB    |

  @fetch @head @owner
  Scenario Outline: Let the owner access stored bytes without a reference
    Given the artifact is stored
    And no accepted event references its digest
    When the owner performs <operation> for the digest
    Then the operation succeeds
    And <disclosure>

    Examples:
      | operation | disclosure                              |
      | GET       | the exact artifact bytes are returned   |
      | HEAD      | no response body is returned            |

  @fetch @head @reference
  Scenario Outline: Let a granted reader access a referenced artifact
    Given an accepted event in stream A references the stored artifact
    And the reader has an active read grant on stream A
    When the reader performs <operation> for the digest
    Then the operation succeeds
    And <disclosure>

    Examples:
      | operation | disclosure                              |
      | GET       | the exact artifact bytes are returned   |
      | HEAD      | no response body is returned            |

  @fetch @head @reference @known-gap-laptop-reference-gating
  Scenario Outline: Deny a reader whose grant does not cover the reference
    Given an accepted event in stream A references the stored artifact
    And the reader has an active read grant only on stream B
    And stream B does not contain the referencing event
    When the reader performs <operation> for the digest
    Then the operation is denied with "scope_denied"
    And no artifact bytes are disclosed

    Examples:
      | operation |
      | GET       |
      | HEAD      |

  @fetch @head @reference @known-gap-laptop-reference-gating
  Scenario Outline: Keep an unreferenced blob invisible to readers
    Given the artifact is stored
    And the reader has an active stream read grant
    But no accepted event references the artifact digest
    When the reader performs <operation> for the digest
    Then the operation is denied with "scope_denied"
    And no artifact bytes are disclosed

    Examples:
      | operation |
      | GET       |
      | HEAD      |

  @authentication
  Scenario Outline: Bind artifact proofs to operation facts
    Given the NIP-98 authorization event has kind 27235
    And its u tag is the full request URL
    And its method tag is "<proof_method>"
    And its payload tag is the SHA-256 of <proof_payload>
    And it has a nonce tag
    When the caller performs <operation>
    Then the proof authenticates only those request facts

    Examples:
      | operation                         | proof_method | proof_payload         |
      | POST /artifacts                   | POST         | the exact upload body |
      | GET /artifacts/{sha256}           | GET          | the empty body        |
      | HEAD /artifacts/{sha256}          | GET          | the empty body        |

  @authentication
  Scenario Outline: Require authentication on every artifact operation
    Given no Authorization header is present
    When the caller performs <operation>
    Then the operation is denied as unauthenticated
    And no artifact bytes are stored or disclosed

    Examples:
      | operation                |
      | POST /artifacts          |
      | GET /artifacts/{sha256}  |
      | HEAD /artifacts/{sha256} |

  @integrity @fetch
  Scenario: Re-verify stored bytes before disclosure
    Given the caller is authorized to fetch the artifact
    And storage under the requested digest contains bytes with a different SHA-256
    When the caller fetches the artifact
    Then the fetch fails
    And the corrupt bytes are not disclosed

  @authentication @head
  Scenario: Authorize HEAD with a GET-signed empty-payload proof
    Given a HEAD request for a stored authorized artifact
    And its NIP-98 event has kind 27235
    And its u tag is the full artifact URL
    And its method tag is "GET"
    And its payload tag is the SHA-256 of the empty body
    And it has a nonce tag
    When the adapter authenticates the HEAD request
    Then the request is authorized
    And no response body is returned

  @validation @sha256
  Scenario: Normalize a valid uppercase digest argument
    Given the artifact sha256 argument is 64 uppercase hexadecimal characters
    When the client or adapter constructs the authenticated artifact operation
    Then it normalizes the digest to lowercase
    And the request URL and proof use the normalized digest

  @validation @sha256
  Scenario Outline: Reject an invalid digest argument
    Given the artifact sha256 argument is <argument>
    When an artifact operation validates the argument
    Then it rejects the argument as invalid
    And it does not disclose artifact bytes

    Examples:
      | argument                         |
      | shorter than 64 characters      |
      | longer than 64 characters       |
      | 64 non-hexadecimal characters   |
