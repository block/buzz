# Launchpad — orientation for this fork

This file is **Launchpad-specific and not upstream**. It lives here rather than in
`AGENTS.md` because that file is maintained by `block/buzz` and changes often;
editing it would create a merge conflict on every upstream sync, forever.

For everything about Buzz itself — architecture, setup, conventions, the quality
gates — read [`AGENTS.md`](AGENTS.md) and [`CONTRIBUTING.md`](CONTRIBUTING.md).
Nothing here replaces them.

## The handbook

Knowledge about deploying, operating and extending *our* Buzz environment does not
live in this repository. It lives in
**[`launchpad-26/handbook`](https://github.com/launchpad-26/handbook)** — a
companion repository publishing an MkDocs site restricted to `launchpad-26`
members.

It answers the questions this repository cannot, because they span several
repositories at once:

- How do *we* deploy Buzz?
- What have we changed from upstream, and why?
- Which operational practices apply to our deployment?
- What should an agent read before working on this part of the system?

The design is [`#4`](https://github.com/launchpad-26/buzz/issues/4), built through
sub-issues [`#6`](https://github.com/launchpad-26/buzz/issues/6)–[`#11`](https://github.com/launchpad-26/buzz/issues/11).

### Why it is a separate repository, and stays one

The handbook **synthesises** several repositories — upstream `block/buzz`, this
fork, `launchpad-26/launchpad`, `launchpad-26/skills`, `launchpad-26/rhizomorph` —
and links back to each source at a pinned commit. A knowledge surface about five
repositories does not belong inside one of them.

**Do not vendor it here as a submodule or subtree.** That would mirror content
instead of synthesising it, which is the fragmentation the handbook exists to
solve, and it would break provenance: every page pins the commit of every source
it drew from, which assumes the repositories move independently.

### Why it is private

Two of its five sources are private. A public site citing them would break its own
evidence chain — a `[cohort]` citation would 404 for anyone outside the org — and
would publish cohort material while every security rule still passed.

### How the two repositories are connected

| Layer | How |
|---|---|
| **Data** | Handbook pages pin commits from this repository; a scheduled job flags a page when a cited file moves |
| **Tracking** | Both repositories are linked to org project **#20 (`buzz`)** |
| **Issues** | Handbook PRs close issues here with `Closes launchpad-26/buzz#<n>` |
| **Code** | **Not connected, deliberately** — see above |

New handbook issues are filed in `launchpad-26/handbook`, which is private. This
repository is public, and content discussion draws on private sources.
