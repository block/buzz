# docker/ — containerised supporting tools

For tools that need pre-configuring to run **outside** the dev environment, so they do not depend
on one person's laptop being set up correctly.

**Empty on purpose.** Nothing #18, #19 or #22 needs lives here, and an unjustified directory
becomes a dumping ground. Create an image when one of the two candidates below actually blocks
something.

## Candidate 1 — Ansible control node

The stronger case. Pins `ansible-core` and collection versions so the plays behave identically on
a Mac, on Linux, and in CI. #37 adds a CI/CD deployment workflow using a dedicated machine
identity; that workflow needs the control environment to be a reproducible artifact rather than
whatever the runner happens to have. Building it before #37 means the pipeline inherits a solved
problem.

Note it must **not** bake in credentials — the container gets an SSH agent socket and an inventory
mounted at run time.

## Candidate 2 — the #39 load generator

#39 builds a load-generation tool as a standalone cargo project with a path dependency on
`crates/buzz-test-client`, and its DoD requires it to run from a machine that is **not** the target
host. Containerising it removes the need for a Rust toolchain wherever it runs, which matters
because that machine is deliberately not the VPS.

Weaker than candidate 1 for now: #39 is off the critical path, and the tool has to exist before
packaging it means anything.

## What does not belong here

- **The Buzz stack itself.** That is `deploy/compose/`, upstream-owned, consumed unmodified.
- **A `tls internal` Caddy override** for local HTTPS. That is a Caddyfile plus a compose override,
  owned by `../ansible/`, not a new image.
- **`buzz-admin`.** Already ships inside the relay image; run it with
  `docker run --rm --entrypoint /usr/local/bin/buzz-admin`.
