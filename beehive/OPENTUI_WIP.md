# OpenTUI functional prototype

Branch: `feat/beehive-opentui-57f9c8fa`, based on
`ec3a22f8ceb4d63d871287c8269a842184fc4ed9` through renderer checkpoint
`d6b34118eb3e610da00878f400cd59e93630cc79`.

Bare packaged `beehive` now opens actual OpenTUI 0.5.11 under Bun 1.4.2.
The controller, existing domain services, host and credential helpers remain Node
24.15.0. `bin/beehive` uses the package's runtime explicitly, not a global runtime.
Arguments still route to the supported existing Node CLI; no renderer/domain fork.

## Usable flows

- Local Host: inspect public retained configuration and local slots, or explicitly
  save missing owner-public/relay configuration through existing host bootstrap.
  No owner sign-in and no implicit service/agent Start.
- Provision a stopped local agent using an existing binding file, owner-authorized
  genesis file and hidden matching agent key, through `provisionCredentialSlot`.
  Existing/partial installations fail closed; no automatic identity reset.
- Agents: saved owner credential sign-in or hidden explicit matching-key import;
  owner verified before OS-only persistence. Sign-out retains the saved key and does
  not Stop hosts/agents. Missing, locked, denied and mismatched keys are not reset.
- Actual private host availability and per-host agent inventory, selectable JSON
  detail with actual run versus selected-next and explicit stale/UNKNOWN wording.
  This is **not an independent agent catalog**. No fabricated inventory rows.
- Select an existing named next configuration; exact host/agent/revision confirmation.
  Explicit Start and Stop through the existing durable management intent client.
  Start requires a fresh assigned stopped report without actual run. Unresolved
  operations fence further lifecycle submissions; inspect/reconcile remains available.
- Selectable operation records include operation ID, exact target, CAS revision,
  receipt/publication and unknown state. Reconcile does not retry blocked work.
- Offline multiline private instruction draft creation, atomically persisted using
  the existing draft API. Publishing/applying remains a distinct existing CLI action.

## Deliberate cuts

- **Restart is unavailable in this prototype, including the controller handler**.
  Existing backend Restart can consume a different selected binding. Rather than
  relying on a disabled button or changing established service semantics in this
  bounded prototype, this manager sends no Restart at all. Explicit Stop, inspect
  accepted stopped state, then Start is available. Existing advanced CLI retains
  its prior explicit Restart behavior; it does not promise same-runtime Restart.
- Foreground host service launch remains `beehive host --owner-present`; Quit this
  manager first. No daemon, process ownership transfer or auto service management.
- Binding/provider authoring remains `beehive local-setup ~/.beehive/host` and the
  existing provision/reconciliation CLI. The new provisioning form accepts already
  prepared binding/genesis artifacts, not a full provider wizard.
- Existing draft editing/resume, profile publication/application, public metadata
  editing/publication and policy retry remain supported through `beehive drafts`
  and `beehive tui discover <relay> ~/.beehive/owner`. New forms explicitly say
  “Not available in this build”; these are not simulated failures or fake success.
- No owner-key generation/reveal UI, independent relay catalog, zero-agent reusable
  runtime catalog, identity-only registration, Move UI or remote provider login.
- For lists/actions longer than their visible pane, use keyboard arrows/Enter.
  Pinned OpenTUI hides its scroll offset: mouse clicks are explicitly refused rather
  than activating an unrelated selected action. No exhaustive UX/polish claim.

## Boundary / cancellation

Node launches only the Bun presentation; two private pipes carry application
requests/snapshots, never owner signers in snapshots. One action is in flight, with
monotonic request ID, captured host/agent/revision and generation fencing. Remote
operation identity, CAS, receipt/ACK and unknown recovery stay in the existing client.

Every explicit synchronous OS credential operation runs in an owned Node helper,
with a 10-second deadline, stripped environment/loaders, no secret argv/output, and
SIGKILL cancellation that settles only after child close. Esc cancels pending local
credential work; cancellation cannot undo an OS write already committed. Partial
configuration/provisioning may retain its existing lock/recovery state; preserve it
and use explicit reconciliation, never delete/reset automatically. Quit restores the
presentation TTY while Node completes owned helper cleanup. Signout/Quit are not Stop.

## Evidence

Task artifacts: `/Users/loganj/.buzz/artifacts/beehive-opentui-57f9c8fa/`.

- Node 24.15 direct TypeScript check passed (`manager-typecheck.log`).
- Focused controller/credential boundary: 4/4 passed (`manager-boundary.log`): real
  config and multiline draft persistence with injected synthetic credentials; owner,
  exact selection, revision and unresolved fences; handler-level Restart refusal;
  duplicate action prevention; late credential completion and signout fencing;
  actual owned Node helper cancellation/exit, no inherited loader.
- Launcher compatibility focused: 5/5 passed. First new controller test invocation
  exposed unsupported Node parameter properties; removed, corrected tests passed.
  Original failure is retained in `manager-focused.log`.
- Actual Bun renderer test: 1/1 passed (`manager-renderer.log`). Previous real renderer
  navigation/hidden input/resize/quit evidence is reused, not an exhaustive new gallery.
- One final default-concurrent package suite: **151 passed, 0 failed, 19 skipped**,
  170 total, natural 28.156s (`manager-full-suite.log`). Installed native opt-ins unset.
  Historical baseline timing failures remain historical, not classified/fixed by this run.
- Brief real-entrypoint PTY: exit 0, exact TTY restore, alternate screen enter/exit,
  real host configuration and multiline draft saved (`worktree-manager.json`). Explicit
  test-only module injection supplies synthetic file credentials and disconnected
  transport. HOME alone is not the credential boundary. No production credential,
  relay/provider/profile or real host lifecycle operation was performed.
- Packaged shell entrypoint and desktop installed evidence are recorded in the final
  delivery artifact/message; the shell wrapper was added after the Node package suite.
  No repository-wide CI or production compatibility claim.

## Reproduce / package

Do not use `pnpm run check`: prior workspace auto-install incident expanded root
packages. Use direct pinned Node commands; do not remove unrelated root node_modules.

```sh
cd beehive
/path/to/node-24.15.0 node_modules/typescript/bin/tsc --noEmit
/path/to/bun-1.4.2 test ./test/opentui-screen.bun.ts
/path/to/node-24.15.0 --test test/manager-controller.test.ts
# Full suite once per final executable candidate, not rerun-to-green:
/path/to/node-24.15.0 --test test/*.test.ts
```

Export the pinned commit's `beehive/` directory, install package-local dependencies
with pnpm 11.4.0 `install --ignore-workspace --ignore-scripts --frozen-lockfile`
(and the command-local approved registry mirror where needed), then place verified
Node 24.15.0 and Bun 1.4.2 executables at `beehive/runtime/node` and
`beehive/runtime/bun`. Link `~/.local/bin/beehive` to the exported package's
`bin/beehive`, not to a worktree. See the immutable delivery recipe for the exact pin.

## Local visual-polish candidate (not installed)

The subsequent presentation candidate replaces default JSON walls with Node-built
public summaries and F3 technical evidence; uses ≥88×24 split / narrow Enter-to-detail;
puts contextual guarded commands in F2; retains permanent keys with F4 scrollable
outcomes; and uses compact validated inputs plus a named configuration chooser.
The installed `6390e594` exports are deliberately unchanged pending independent
implementation-vs-design review.

Evidence and the corrected external design are under
`/Users/loganj/.buzz/artifacts/beehive-opentui-polish-6390e594/implementation/`.
Read `IMPLEMENTATION.md` there for the eight-point self-review, scoped deviations,
actual PTY comparisons versus offscreen fixtures, exact runtime commands and limits.
This candidate does not claim exhaustive design conformance, real relay inventory,
light-terminal visual proof, or safe scrolled-action mouse activation.
