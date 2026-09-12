# Staging dev relay images

Use the **Staging dev relay image** GitHub Actions workflow to publish a pre-merge Buzz relay runtime image for bb-block staging.

1. Run `.github/workflows/staging-dev-relay-image.yml` from the default branch.
2. Enter a `target_ref` from `block/buzz` (`my-branch`, `refs/heads/my-branch`, `my-tag`, or `refs/tags/my-tag`).
3. Wait for the workflow summary. It resolves that ref to a commit and publishes only the relay `runtime` image to:

   ```text
   ghcr.io/block/buzz-staging-dev:dev-sha-<40-character-commit-sha>-run-<run-id>-<run-attempt>
   ```

4. In `squareup/builderbot-platform-core-infrastructure`, set bb-block staging values to the distinct pull-through ECR path and immutable tag from the workflow summary:

   ```yaml
   buzz:
     image:
       repository: 929862310821.dkr.ecr.us-west-2.amazonaws.com/ghcr.io/block/buzz-staging-dev
       tag: dev-sha-<40-character-commit-sha>-run-<run-id>-<run-attempt>
   ```

This path is intentionally separate from the production/main relay image path (`ghcr.io/block/buzz`) so a staging-only branch deployment is obvious in BPCI.

These images are for manual, pre-merge staging evaluation only. They are not release-qualified and must not be promoted to production or used by the canonical Kargo promotion path.

While bb-block remains a shared staging environment, do not deploy a target ref through this runtime-only path if it changes `migrations/` or requires changes to `deploy/charts/buzz/`. Backwards-compatible migrations can still leave the shared database ahead of the restored `main` image. Use an isolated environment or deployment-time enforcement for those changes.
