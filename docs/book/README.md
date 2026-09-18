# Docs book

Renders the Markdown already tracked in this repo as a browsable book, using
[mdBook](https://rust-lang.github.io/mdBook/). The Markdown stays the source of
truth — this directory holds only the config and the script that assembles it.

## Local preview

```bash
cargo install mdbook          # or: brew install mdbook
python3 docs/book/build.py
mdbook serve docs/book
```

## How pages are chosen

Pages are **discovered, not listed**. Every tracked `.md` file is published
unless it matches `EXCLUDE` in `build.py`, so a new document appears in the
book by existing. The inverse — an explicit include list — fails silently: a
file nobody remembered to add looks exactly like a file that was never
written.

`EXCLUDE` covers machine-facing Markdown: agent instructions, prompt
fragments, persona definitions, skill manifests, issue templates and test
fixtures.

Placement comes from `SECTIONS`, an ordered list of path patterns, first match
wins. Anything matching no rule is still published, under **Unsorted**, and
named on stdout. `--check` turns that into a build failure, which is what CI
runs — a new document has to be placed deliberately rather than drifting into
a catch-all.

Section ordering is derived from `SECTIONS` itself, so adding a rule is the
only step; there is no second list to keep in sync.

## Why files keep their repo paths

Documents are copied into `src/` at their original paths. Relative links
between them (`[NIP-AE](NIP-AE.md)`, `[architecture](../../ARCHITECTURE.md)`)
therefore resolve unchanged, because the distance between any two files is
preserved, and mdBook rewrites `.md` targets to `.html` at build time.

Three cases still need handling, in `resolve_links()`:

- mdBook renders `README.md` as `index.html`, so links *to* a README are
  rewritten to `index.md`.
- Links to excluded files are repointed at GitHub, where they do exist.
- Anything else is already broken in the repo. Those are reported on stderr
  and left alone — the fix belongs in the document.

That last report currently lists 36 links, all of them bare-number references
to the upstream nostr-protocol NIPs repo (`01.md`, `44.md`, and similar). They
do not resolve on GitHub either.
