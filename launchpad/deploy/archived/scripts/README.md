# scripts/ — helpers

Helpers that are not tied to one hypervisor and do not converge a host toward a desired state.
If a script only makes sense against VirtualBox it belongs in `../virtual-box/`; if it configures
a host it belongs in `../ansible/`.

Originally scoped as "runs on a target host", but `resolve-image-tag.sh` runs on the **control
node** against the repo and the registry. Both kinds live here rather than splitting hairs with a
fourth directory — the boundary that matters is hypervisor-specific vs. converging vs. neither.

## Contents

### `resolve-image-tag.sh`

Prints the `BUZZ_IMAGE=` line pinning `ghcr.io/block/buzz` to the image matching the
`deploy/compose` bundle in a fork checkout.

The relay runs from a prebuilt upstream image while the bundle is deployed from the fork, so those
two things can drift. Pinning to whatever upstream built most recently pairs new relay code with
older config — the naive "newest `sha-` tag" approach is wrong. The correct pin is the upstream
commit the fork is **synced to**, found via `git merge-base` against `block/buzz` main.

It refuses to answer if `deploy/compose` differs between your ref and the sync point, because then
no upstream image matches what you are actually running. That doubles as a check on
`launchpad/AGENTS.md` section 3, which forbids editing `deploy/compose/`. If the sync-point commit
has no published image it walks back along first-parent, accepting only commits whose bundle is
identical to the sync point's.

```bash
./scripts/resolve-image-tag.sh /path/to/buzz HEAD
# -> BUZZ_IMAGE=ghcr.io/block/buzz:sha-96ae141
```

Current answer for this checkout: fork `b11ca33e8` syncs to upstream `96ae14176` (2026-08-05),
bundle identical, so **`sha-96ae141`**, digest `sha256:472e9cf7…`.

Verified by running the happy path. The bundle-differs guard and the walk-back branch have **not**
been exercised — testing them needs a checkout in those states.

## Still wanted

What the open issues call for:

- **Measurement capture** for #18, #20 and #39 — peak RSS per container, total host memory, swap
  used against the 496 MB available, disk consumed. #18 wants these at four distinct points
  (`docker pull`, migration, first relay start, steady idle), which is fiddly enough by hand to be
  worth scripting, and #39 needs an idle baseline captured before any load figure is trusted.
  A script also makes the method reproducible by a reviewer, which several DoDs require.
- **Preflight checks** — Compose ≥ 2.24.4, disk headroom, swap present and sized, no `CHANGE_ME`
  left in `.env`.

Deliberately **not** here: anything that writes measurement results into the repo. #18 and #20
both state the issue produces evidence, not code — output goes to stdout and onto the GitHub
issue.
