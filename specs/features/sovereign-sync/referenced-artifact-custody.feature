# id: sovereign-referenced-artifact-custody
# type: feature
# story: specs/stories/sovereign-sync/retrieve-referenced-stream-artifacts.md
# journey: specs/journeys/fetch-referenced-stream-artifacts.md
# model: specs/models/sovereign-sync/sovereign-stream-agreement.model.yaml
# contract: HEAD /artifacts/{sha256}, GET /artifacts/{sha256}

@sovereign-sync @artifacts
Feature: Retrieve artifacts through accepted event references
  As a sovereign node operator
  I want blob access to follow readable event manifests
  So that artifact custody never becomes broader than the stream that introduced it

  Background:
    Given artifact identity is its lowercase SHA-256 digest
    And accepted signed events may reference artifacts with x tags

  @happy-path
  Scenario: Fetch and verify an artifact referenced by a readable stream
    Given an event in a stream readable by transport principal B1 references artifact X
    And artifact X is present at the custodian
    When B1 fetches artifact X
    Then the custodian returns its bytes
    And the destination verifies the bytes hash to X before storing them

  @authorization
  Scenario Outline: Do not disclose a blob without an authorized reference path
    Given <condition>
    When transport principal B1 checks or fetches artifact X
    Then artifact X is not disclosed
    And no blob bytes are stored locally

    Examples:
      | condition                                                        |
      | no accepted event references artifact X                          |
      | only an event in a stream unreadable by B1 references artifact X |
      | B1's read grant has been revoked                                 |

  @integrity
  Scenario: Reject bytes that do not match the requested digest
    Given an authorized event references artifact X
    When the custodian returns bytes whose SHA-256 is not X
    Then the destination reports an integrity failure
    And the invalid bytes are not stored under X

  @idempotency
  Scenario: Treat an already present verified artifact as complete
    Given verified bytes for artifact X already exist locally
    When artifact synchronization encounters another reference to X
    Then no duplicate logical artifact is created
    And synchronization continues without changing event cursors

  @diagnostics
  Scenario Outline: Distinguish artifact failure outcomes
    Given a readable event references artifact X
    When artifact synchronization encounters <failure>
    Then it reports "<outcome>"

    Examples:
      | failure                           | outcome              |
      | no read authority                 | authorization_denied |
      | custodian has no bytes            | artifact_unavailable |
      | returned bytes fail hash checking | integrity_failure    |
