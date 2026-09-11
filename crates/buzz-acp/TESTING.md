# Pi adapter integration

Buzz's Pi preset uses [salman1993/pi-acp](https://github.com/salman1993/pi-acp).
Requires Node.js 22 or newer. Install Pi and configure its model provider,
then install the adapter directly from the fork:

```sh
npm install -g @earendil-works/pi-coding-agent
pi
npm install -g --install-links=true git+https://github.com/salman1993/pi-acp.git#main
```

Restart Buzz, then select **Pi** as the agent harness. Buzz starts `buzz-pi-acp`
automatically. Run the same adapter install command again to update it.
The unscoped `npm install -g pi-acp` command installs the upstream package,
without these extensions. Use fresh sessions to replace old user-framed
standing instructions.

Buzz adds `-- --skill <harness-cwd>/.agents/skills` when launching `buzz-pi-acp`.
An existing separator and explicit Pi options are preserved. Managed agents
run from the Buzz nest (normally `~/.buzz`): Desktop sets the `buzz-acp` child
CWD through `default_agent_workdir()`, and adapters inherit it. The default
skill directory is that launch workspace's `.agents/skills`. Standalone CLI
launches use the caller's working directory.
The path is fixed at adapter launch and applies to every Pi subprocess.

The full composed session prompt is sent as a replacement string through the
provisional `session/new.params._meta.systemPrompt` field. Buzz recognizes
`buzz-pi-acp` by the agent name returned during initialization, regardless of
protocol version. Upstream `pi-acp` does not receive this fork-specific metadata.
No custom capability negotiation or legacy Pi prompt fallback is used.
Session titles continue to use `_meta.sessionTitle`.

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
