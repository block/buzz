# AGENTS.md — Launchpad deployment guard and failed deployment archive

The deployment method formerly stored in this directory **failed** and has been
moved to `archived/` for historical reference only. The active `run.sh` in this
directory is a thin Launchpad image guard that delegates all orchestration to
`../../deploy/compose/run.sh`; it is not a replacement Compose implementation.

## Mandatory rule

**Do not use, run, copy, repair, extend, or recommend anything in `archived/` to
build or deploy Buzz.** The archive is an example of what does not work, not an
inactive but supported deployment option.

The failed method was developed without a clear trace of the complete VPS
deployment path. It mixed fork-local automation with upstream deployment files
and ultimately selected the Block test relay image through the hard-coded
`ghcr.io/block/buzz:main` default. As a result, a checkout of
`launchpad-26/buzz` could deploy code built from `block/buzz` rather than code
from the Launchpad repository. This ambiguity caused significant operator
confusion and wasted agent effort.

The archived material may be read to understand the failure, recover general
research, or avoid repeating its mistakes. It is not a source of deployment
truth. Do not infer that a command is safe merely because it remains in the
archive.

Before any future Launchpad deployment work begins, trace image publication and
runtime image selection end to end and prove that the selected immutable image
was built from the intended `launchpad-26/buzz` commit. The saved audit in this
directory records the repository-mapping problem discovered in this attempt.
