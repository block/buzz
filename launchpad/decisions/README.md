# Decision records

Accepted ADRs live here. One file per decision, named `ADR-XXXX-slug.md`, numbered in the
order they were accepted — not in issue order, and numbers are never reused.

## The lifecycle

Per [`../AGENTS.md`](../AGENTS.md):

1. An open question becomes a **`type:adr` issue**, parented to the PRD that raised it.
2. The issue is where the decision is argued. Its *Decision outcome* stays blank until a
   human settles it — agents draft, they do not decide.
3. When it is settled, the decision is written here **in the same pull request that closes
   the issue**. Closing without the document is not done: a decision recorded only in a
   closed issue is lost to the noise.

## What a record contains

The issue holds the full argument. This file holds what was decided and enough context to
understand it without reading the issue.

| Section | Purpose |
|---|---|
| Frontmatter | `status`, `date`, `issue`, and `decided_in` where the decision actually happened |
| Decision | What was chosen, stated plainly. The load-bearing section. |
| Context | Why the question existed at all. |
| Consequences | What follows, good and bad. State the bad honestly. |
| Provenance | Where the decision was really made, if it predates its ADR issue. |

Superseding a decision does not edit it. Write a new record, set the old one's `status` to
`Superseded by ADR-YYYY`, and say so in the new record's `Supersedes`.
