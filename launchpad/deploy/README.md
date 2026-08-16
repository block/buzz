# Launchpad VPS deployment guard

Use [`run.sh`](run.sh) for Launchpad VPS operations. It validates that
`deploy/compose/.env` selects an immutable `ghcr.io/launchpad-26/buzz` relay
image, rejects upstream Block images, checks Docker Compose compatibility, and
then delegates to the canonical `deploy/compose/run.sh` implementation.

```bash
./launchpad/deploy/run.sh check
./launchpad/deploy/run.sh start
./launchpad/deploy/run.sh upgrade
```

Digest references and full 40-character `sha-...` tags are accepted for normal
deployment. Floating tags are rejected unless
`BUZZ_ALLOW_FLOATING_IMAGE=true` is explicitly set for development or testing.
The override never permits `ghcr.io/block/buzz`.

The guard reads exactly one `BUZZ_IMAGE` assignment from the local `.env` and
exports that value before delegation, so an ambient shell variable cannot
silently replace the reviewed deployment image.

## Failed deployment method — archived

Everything from the former Launchpad VPS deployment experiment has been moved
to `archived/` for future reference.

> **Do not use the archive to build or deploy Buzz.** It documents a failed
> method and is retained as an example of what does not work.

The method failed because the complete deployment file and image-selection path
was not understood before automation was built around it. The resulting process
mixed Launchpad-specific work with upstream deployment behavior and defaulted to
the hard-coded Block test relay image, `ghcr.io/block/buzz:main`. It therefore
did not reliably deploy code from `launchpad-26/buzz` and was difficult for both
the operator and collaborating agents to redirect back to the Launchpad
repository.

The archive remains useful for postmortem analysis and as a source of lessons.
It is not maintained, supported, or approved for execution. See
`VPS-DEPLOYMENT-AUDIT.md` for the file-path and image-selection audit that
confirmed the failure.
