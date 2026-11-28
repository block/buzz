# Account settings review lab fixture — DO NOT MERGE

This draft is a disposable routing and reviewer test. All examples are documentation,
contain deliberate defects, and must never be copied into production.

## Review areas

- [Design: validation feedback](design/account.md)
- [Backend: account isolation](backend/account.md)
- [UI: save state](ui/account.md)

## Routing

Use the `design`, `backend`, and `ui` PR labels to test label rules, or use these
file-path rules for corresponding lanes:

- Design: `docs/review-lab-fixture/design/**`
- Backend: `docs/review-lab-fixture/backend/**`
- UI: `docs/review-lab-fixture/ui/**`

Each area should match independently. Reviews belong only in the test lab's Buzz
threads. Do not post automated reviews to GitHub, merge this draft, or change
application behavior. Submitting the draft to the lab is a manual test step.
