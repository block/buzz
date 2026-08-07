# Fork sync — keep our patches on top of `block/buzz`

This machine’s product surface is the fork `radu2lupu/buzz`, with local
patches that must survive every upstream pull:

1. **Native Grok Build ACP** runtime discovery / onboarding
2. **Smart thread participation** (bare human replies in active threads)
3. **Fieldcraft env injection** into managed agents

## Remotes

| Remote     | URL                              | Role                          |
|------------|----------------------------------|-------------------------------|
| `upstream` | `https://github.com/block/buzz`  | Canonical source of truth     |
| `origin`   | `https://github.com/radu2lupu/buzz` | Our fork (push target)     |

## Branches

| Branch                 | Meaning                                              |
|------------------------|------------------------------------------------------|
| `main`                 | Fast-forward only of `upstream/main`                 |
| `agent/native-grok-acp`| Our patch stack rebased onto current `main`          |

Never commit product patches on `main`. They live only on the patch branch
(or a successor with the same role).

## Standing rule

**When Radu says “update to latest”:** run the sync script, rebuild the
participation harness, reinstall it outside the signed app, restart agents.
Do **not** install stock desktop alone and drop our harness.

```bash
./scripts/sync-upstream.sh          # fetch + rebase patches
./scripts/install-participation-acp.sh   # release build → agents/bin
# then restart Buzz so agents pick up the new binary
```

## What the scripts do

### `scripts/sync-upstream.sh`

1. `git fetch upstream main` (and `origin`)
2. Reset local `main` to `upstream/main`
3. Rebase `agent/native-grok-acp` onto that `main`
4. Print conflict guidance if rebase stops
5. Optionally `git push origin main` and force-with-lease the patch branch
   (`--push`)

### `scripts/install-participation-acp.sh`

1. `cargo build --release -p buzz-acp` on the patch branch
2. Install to  
   `~/Library/Application Support/xyz.block.buzz.app/agents/bin/buzz-acp`
3. Ad-hoc sign that binary (outside the app bundle so Gatekeeper stays happy)
4. Ensure active managed agents with a `relay_url` point `acp_command` at that
   path (never hot-swap into `/Applications/Buzz.app`)

## Why not edit the app bundle

Replacing `Contents/MacOS/buzz-acp` inside the notarized app breaks the
Developer ID signature → “Buzz is damaged…”. Our harness always lives under
Application Support; Desktop resolves absolute `acp_command` paths.

## Mini relay

The Mac Mini runs **upstream images**, not this fork’s tree:

```bash
ssh mini 'export DOCKER_HOST=unix://$HOME/.colima/default/docker.sock
  cd ~/Projects/buzz/deploy/compose
  docker pull ghcr.io/block/buzz:main
  ./run.sh up -d --force-recreate --no-deps relay pair-relay'
```

Agent conversational patches (participation) are **desktop/harness**, not
relay. Updating the mini is independent of rebasing this branch.

## Conflict policy

- Prefer **upstream** for generic refactors; re-apply our hooks.
- Keep `participation.rs` and the `thread_participation_*` desktop modules
  intact unless upstream added an equivalent feature.
- After resolving, re-run:  
  `cargo test -p buzz-acp participation`

## First-time setup on a new clone

```bash
git remote add upstream https://github.com/block/buzz.git
git fetch upstream main
git checkout main && git reset --hard upstream/main
git checkout agent/native-grok-acp
./scripts/sync-upstream.sh
./scripts/install-participation-acp.sh
```
