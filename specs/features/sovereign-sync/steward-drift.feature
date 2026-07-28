# id: sovereign-steward-drift
# type: feature
# stories: specs/stories/sovereign-sync/observe-and-report-stream-drift.md, specs/stories/sovereign-sync/reconcile-stream-agreement-drift.md
# journey: specs/journeys/detect-and-reconcile-stream-drift.md
# model: specs/models/sovereign-sync/sovereign-stream-agreement.model.yaml
# contract: POST /query, POST /events

@sovereign-sync @steward @drift
Feature: Observe and reconcile sovereign stream drift
  As a sovereign node operator
  I want bounded, attributable drift observations
  So that only owners decide how trust changes

  Background:
    Given a steward has its own signing key
    And owner-signed steward mandates are effective addressable heads

  @mandate @observe
  Scenario: Observe only the nodes granted by active mandates
    Given an active mandate names the steward and grants observe for node A
    And no active observe mandate covers node B
    When the steward evaluates declaration and health state
    Then its findings are limited to node A
    And it receives no declaration-writing authority

  @mandate @revocation
  Scenario Outline: Refuse work without current observe authority
    Given the only mandate naming the steward is <mandate>
    When the steward starts
    Then it performs no observation or publication

    Examples:
      | mandate                    |
      | revoked                    |
      | missing observe power      |
      | authored by a non-owner    |
      | scoped only to another key |

  @mandate @report
  Scenario: Publish only under report power for the same scope
    Given one mandate grants observe and report for node A into context H
    And another mandate grants observe only for node B
    When the steward finds drift on both nodes
    Then it may publish node A findings with h tag H
    And node B findings remain local only

  @classification
  Scenario Outline: Classify current declaration mismatch
    Given the relevant current declaration heads have <condition>
    When the steward evaluates agreement state
    Then it reports "<finding>"

    Examples:
      | condition                                       | finding   |
      | an export and no admit                          | open      |
      | an admit author not offered by the export       | unoffered |
      | an offered admit with no export pin             | unpinned  |
      | an offered admit pinned to the old export head  | drifted   |
      | a required declaration whose head is revoked    | inactive  |

  @deduplication
  Scenario: Repost only materially changed findings
    Given the steward previously published its current scoped findings
    When the same scoped findings are evaluated again
    Then no new report is published
    When a current head or health outcome changes
    Then a new signed report may be published

  @reconciliation
  Scenario: Require an owner decision to repair drift
    Given the steward reports a stale admit pin
    When the destination owner reviews and accepts the current export
    Then only the destination owner publishes the replacement admit
    And the steward can report the resulting matched state
