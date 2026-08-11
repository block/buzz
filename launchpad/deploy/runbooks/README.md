# runbooks/ — committed procedures

Procedures a reviewer can follow without having been in the room. Two open issues require
artifacts that land here:

- **#19** — "Produce a runbook, committed under `launchpad/deploy/`, that takes a fresh relay to a
  state where a chosen hostname resolves to a seeded community and only rostered pubkeys can
  connect."
- **#38** — document bootstrap, rotation, rerun and host recovery.

Distinct from `../scripts/`: a runbook explains and records decisions, including the ones a script
cannot make (which hostname the cohort uses, who is on the roster).

## Contents

| File | Status |
|---|---|
| `dev-deployment-SOP.md` | **Start here.** Nothing to a working dev environment, one step at a time, no scripts. Beginner audience |
| `relay-build-list.md` | Why the pieces behave as they do — source references and the reasoning behind the SOP's steps |
| `hardening-spec.md` | Specification for the PRD #5 hardening roles (#29–#34). Nothing implemented. Buzz-specific findings read from source, the host baseline, and the dev/prod parity model |

`dev-deployment-SOP.md` is also the **specification the automation is written against**. If an
Ansible role does something the SOP does not describe, one of the two is wrong. Changes to how the
environment is built belong in the SOP first, and the roles follow. It closes with an explicit list
of which steps have been executed and which have only been derived from source — keep that list
honest, because it is what tells a reader how far to trust each section.

`relay-build-list.md` is not yet the #19 runbook. It establishes the mechanism, including the
finding that **no community-seeding command exists** and that `RELAY_URL` alone controls which
`Host` headers the relay accepts. #19's runbook builds on it by recording the rehearsal itself with
raw output, plus the hostname the cohort commits to.

Evidence does **not** belong here. #18 and #20 both state their issue "produces evidence, not
code" — capacity reports and measurements are posted as issue comments.
