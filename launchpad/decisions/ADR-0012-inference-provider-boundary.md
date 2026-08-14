---
status: Proposed
date: 2026-08-15
issue: launchpad-26/buzz#53
decided_in: launchpad-26/buzz#53
supersedes: none
---

# ADR-0012 — Inference provider boundary and credential handling for upstream synthesis

## Decision

**One HTTP inference endpoint plus a model identifier, both configuration** — with
**OpenRouter as the MVP value of that configuration**. No named vendor SDK is compiled
into the tool, and no model name is treated as an architectural commitment: Ruling 5
requires the model stay swappable for cost and reasoning quality without a code change,
so pinning it here would freeze the one variable the ruling wants free. The credential
follows the same rule #25 already established for the deployment identity: it lives in
Actions secrets or an Environment, never in a tracked file.

Model selection, prompt text, and token budgets remain implementation detail and are not
recorded here.

## Context

Ruling 5 states the capability "must not depend on one specific inference model" and
names OpenRouter as a possible MVP inference path. #3's Security implications section
says "the provider choice and any API credential become part of this repo's security
surface. Credentials must live in Actions secrets or an Environment, never in tracked
files" — `launchpad/AGENTS.md` section 8 says the same in general terms for this public
repository. Two things make this an architectural commitment rather than implementation
detail: the credential's blast radius on a public repo, and the substitutability seam
Ruling 5 already requires to exist somewhere in the code.

#25 governs the *deployment* identity and its secret storage; the inference credential
is a different credential with a different blast radius and does not inherit that
decision automatically — this ADR states its custody separately, on the same underlying
rule.

## Consequences

**Good.** Separating the committed part (credential custody, egress destination,
substitutability seam) from the free part (model, prompts, budgets) means the cohort can
change models weekly without reopening a decision, while the security-relevant half is
written down once and reviewable. It gives the first implementation task a concrete
statement of what it may and may not hard-code.

**Bad, stated honestly.** An abstraction fixed before the first report is written will be
fixed at the wrong place in some respect, and a seam maintained for substitutability that
is never exercised is cost paid for optionality nobody uses. Routing through an
aggregator adds a party — its availability and its terms become the capability's
availability and terms. Naming where the credential lives creates an operational
obligation (rotation, an owning account, revocation on cohort exit) that someone has to
hold beyond the milestone.

**Contingency 1 — the egress scope boundary does not silently expand.**

*Trigger:* implementation of Ruling 7 (fork-aware upstream analysis) begins — any change
that would send this repo's deployed version, local divergence, or risk assessment to
the inference provider, rather than only public upstream content.

*The fix:* that expanded egress requires its own ADR before it ships. This ADR's
credential-custody and architecture decisions cover Phase 1's public-content egress
only; they do not automatically extend to Ruling 7's egress.

*The safety net in the meantime:* the PR implementing Ruling 7 must reference this ADR
by number and cannot merge without either a superseding ADR or an explicit amendment
recorded against it.

**Contingency 2 — spend is an availability control, not just an accounting one.**

*Trigger:* unexpected volume or cost on the OpenRouter account — an unattended scheduled
job with an unmetered key is a denial-of-service against the cohort's own budget.

*The fix:* a provider-side spend cap or budget alert configured at account creation; on
trip, rotate or revoke the key and pause the trigger until reviewed.

*The safety net in the meantime:* the account owner (@serina-mcfall, confirmed in
conversation) reviews spend periodically. This is recorded here so it is not assumed to
be automatic.

## Security implications

**Credential custody.** The key must be in Actions secrets or an Environment, never in a
tracked file — #3's Security implications and `launchpad/AGENTS.md` section 8 both
require it, and the repository being public makes the failure irreversible rather than
embarrassing. On a public repository the fork-PR path matters as much as the storage:
any workflow reachable from a fork that can read the key publishes it. #25 records
exactly this reasoning for the deployment identity; the same reasoning applies here to a
different credential, and the two must not share a secret.

**Egress.** Phase 1 sends public upstream content to a third party, which discloses
nothing confidential. Ruling 7 changes that — see Contingency 1 above.

**Least privilege and spend.** The synthesis step needs no repository write access
beyond publishing its output, and per #3 should declare minimal `permissions:` explicitly
rather than inherit defaults. See Contingency 2 above for spend as an availability
control.

**Provenance.** Ruling 3 is the control that keeps a fabricated claim detectable.
Whatever provider arrangement is used must preserve source links through synthesis; a
provider integration that returns prose without carrying evidence identifiers through
would remove the only mechanism that makes the output checkable.

## Provenance

Decided directly in conversation with the repository owner (@serina-mcfall) on
2026-08-15, following the recommendation posted as a comment on #53 (2026-08-14) and the
contingency plan posted as a follow-up comment on #53 (2026-08-15) — the same pattern
used for [ADR-0008](./ADR-0008-security-audit-privilege.md),
[ADR-0009](./ADR-0009-upstream-intel-phase-1-scope.md), and
[ADR-0011](./ADR-0011-external-security-smoke-test-floor.md). `issue` and `decided_in`
both point to #53 because the decision and its filing issue are the same place.

Not verified independently in this document: OpenRouter's API shape, terms, or
data-retention policy, and whether the cohort's OpenRouter account and spend limit exist
yet at the time of writing — recorded as the account owner's responsibility going
forward, not verified as already in place.
