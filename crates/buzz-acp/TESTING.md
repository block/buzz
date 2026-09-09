# Pi adapter integration

Buzz's Pi preset uses [salman1993/pi-acp](https://github.com/salman1993/pi-acp).
Install Pi separately, then build and install the adapter from source:

```sh
npm install -g --ignore-scripts @earendil-works/pi-coding-agent
git clone https://github.com/salman1993/pi-acp.git
cd pi-acp
git checkout b893ff9241c35fd04f27b0e9dbfa2f7bc463fc42
npm ci
npm run build
npm install -g .
```

Keep that checkout if npm links the global executable to it. Direct GitHub npm
installation at this revision does not build `dist/index.js`. The unscoped
`npm install -g pi-acp` command installs the upstream package, without these
extensions. Restart managed Pi agents after installing the fork; use fresh
sessions to replace old user-framed standing instructions.

Buzz adds `-- --skill <harness-cwd>/.agents/skills` when launching `pi-acp`.
An existing separator and explicit Pi options are preserved. Managed agents
run from the Buzz workspace, so the default directory is its `.agents/skills`.
The path is fixed at adapter launch and applies to every Pi subprocess.

The full composed session prompt is sent as a replacement string through
`_meta.systemPrompt` only when Pi advertises both `replace` and `persisted`
under `agentCapabilities._meta.piAcp.systemPrompt`. Older adapters retain
first-turn user framing. Session titles share `_meta.sessionTitle`.

## Validation

Activate Hermit from the Buzz repository root, then run the package tests:

```sh
. ./bin/activate-hermit
cargo test -p buzz-acp
```

To exercise the real adapter through Buzz's production session composer:

```sh
BUZZ_TEST_PI_ACP=/absolute/pi-acp/dist/index.js \
  cargo test -p buzz-acp real_pi_preserves -- --ignored
```

This test requires Node and Pi on PATH. It isolates HOME and Pi settings,
disables extensions and context files, and uses a synthetic transcript without
model calls. It inspects Pi's effective prompt through RPC HTML export after
switching sessions and restarting the adapter. Base, persona, team, core memory,
huddle, canvas, and the extra skill must each appear once, without another
session's instructions or Pi's default coding preamble.

HTML export reports the exporting process's current system prompt. It cannot
recover a historical prompt from an old transcript alone.
