# OpenTUI implementation checkpoint — NOT an install candidate

Base: ec3a22f8ceb4d63d871287c8269a842184fc4ed9.
Branch: feat/beehive-opentui-57f9c8fa.

Normal launcher and CLI are deliberately unchanged. This is only the new renderer,
not the requested working management vertical slice. Do not install or publish it
as the owner candidate. The source-author and held Blessed worktrees are unchanged.
No Blessed dependency or source was copied into this worktree.

## Implemented

- Actual pinned @opentui/core 0.5.11, standalone pnpm lockfile.
- `src/opentui-screen.ts`: two always-visible destinations, selectable list/detail,
  detail ScrollBox, action/help bar, focus traversal, text/multiline/hidden forms,
  explicit typed confirmation, busy guard, small-terminal guard, resize and quit.
- Secret entry is application-owned volatile input, never an ordinary Input or
  Textarea and never rendered, including length. Close/cancel clears its reference.
- Reusable honest unavailable notice and consistent signed-in/key-saved header.
- `test/opentui-screen.bun.ts`: actual OpenTUI test renderer, synthetic data only.
- `test/opentui-pty-fixture.ts`: actual alternate-screen fixture, no imports of
  credentials/config/transport/provider/host factories. This is not manager routing.

## Validation of this checkpoint workstate

Artifacts: `/Users/loganj/.buzz/artifacts/beehive-opentui-57f9c8fa/`.

Official Bun 1.4.2 darwin-aarch64 downloaded from the oven-sh/bun GitHub release;
zip verified against that release's published SHASUMS256.txt. This establishes
checksum equality, not independent signature verification. Runtime remains app-local
in the artifact directory; no global runtime or launcher changed.

- Node 24.15.0 `node node_modules/typescript/bin/tsc --noEmit`: passes.
- Bun 1.4.2 `bun test ./test/opentui-screen.bun.ts`: 1 pass, 0 fail. Covers real
  rendered list selection, hidden typing/paste cancellation, multiline save,
  60x24 -> 30x10 -> 60x24 resize, pending-input quit.
- `pty-smoke.py`: real PTY launch/navigation/forms/resize/quit; exit 0, exact tty
  flags restored, alternate-screen enter/exit present, synthetic secret absent.
  Raw transcript and JSON result retained. Not an installed launcher test.
- No full package suite run yet: functional integration is not complete. Run it
  once on the completed functional state, not to label this renderer a candidate.
- Initial Bun test invocation without `./` matched no tests; original log retained.
- Important tooling incident: `pnpm run check` triggered pnpm 11 auto-install of
  the root workspace (614 cached packages). No root tracked file changed, and no
  global/Hermit bootstrap was requested. Avoid `pnpm run` here; direct pinned Node
  invocation above bypassed auto-install. Do not repeat or clean unrelated files.

## Required continuation (no owner approval gate)

1. Complete typed manager controller and real operations. Keep manager/service and
   credential-helper interpreter Node; use Bun for OpenTUI presentation only unless
   a separately validated narrow boundary proves safe. Baseline helper explicitly
   spawns `process.execPath`; merely importing existing manager under Bun silently
   changes that interpreter and is not accepted. Held Blessed controller has useful
   inventory/lifecycle ideas but synchronous credential calls are not suitable.
2. Wire existing host configuration/provisioning, owner sign-in, host inventory,
   lifecycle/configuration, offline drafts and explicit profile/public publication.
   Missing independent catalog/runtime architecture must remain honestly unavailable,
   while existing supported domains remain usable. Never insert fixture rows into
   production. Generated one-time reveal lifecycle is not implemented here.
3. Enforce mismatched Restart refusal in governing host admission, not just UI.
   Baseline host.ts captures selected-next at restart; binding-change tests currently
   expect the old runtime-switch behavior and need deliberate correction.
4. Complete mouse selection for scrolled lists/actions; current click mapping only
   selects exact rows when the entire list fits, otherwise selects the current row.
   Check focus containment and long-label layout. Small-terminal modal rendering
   is not exhaustively assessed. This is prototype work, not a new review gate.
5. Add narrow Node/Bun bridge, fixture seam and launcher routing. Keep supported
   advanced CLI accessible. Install a frozen isolated dedicated package and pinned
   app-local runtime, then validate the actual installed launcher with explicit fake
   credentials/transport (temporary HOME alone is unsafe on macOS).
6. Focused boundary regressions/typecheck, one final package suite, brief PTY smoke,
   self-review, signed-off commit with verified Logan identity, authorized new-branch
   publication (no PR/default mutation), exact remote pin, desktop installation and
   bounded laptop recipe. Nothing has been published or installed by this checkpoint.

## Reproduce local renderer evidence only

```sh
cd /Users/loganj/.buzz/REPOS/beehive-opentui-57f9c8fa/beehive
/Users/loganj/Library/Caches/hermit/pkg/node-24.15.0/bin/node node_modules/typescript/bin/tsc --noEmit
/Users/loganj/.buzz/artifacts/beehive-opentui-57f9c8fa/runtime/bun-darwin-aarch64/bun test ./test/opentui-screen.bun.ts
python3 /Users/loganj/.buzz/artifacts/beehive-opentui-57f9c8fa/pty-smoke.py
```
