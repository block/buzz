# AGENTS.md — historical material only

This entire subtree is an archive of a **failed deployment method**.

**Do not execute, copy, repair, extend, or recommend any command, playbook,
script, configuration, or procedure in this subtree to build or deploy Buzz.**
No file below this point is an active instruction or a source of deployment
truth, even when its historical wording says otherwise.

The method failed because its deployment dependency and image-selection path
was not understood end to end. It could silently select the hard-coded
`ghcr.io/block/buzz:main` test relay image instead of an image built from the
intended `launchpad-26/buzz` commit. The material is retained only for
postmortem reference and to show what must not be repeated.
