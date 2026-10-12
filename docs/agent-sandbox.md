# Agent sandbox

Buzz Desktop runs every local agent inside a [nono](https://nono.sh) sandbox on
macOS and Linux. The sandbox is applied by the operating system (Seatbelt on
macOS, Landlock on Linux) to the agent harness and everything it starts: the
agent CLI, its MCP servers, and every shell command the agent runs.

Design notes and implementation status: [nono-agent-sandbox.md](nono-agent-sandbox.md).

## What agents can and cannot reach

Agents **can**:

- Read and write their workspace, the Buzz nest (`~/.buzz`; `~/.buzz-dev` for
  dev builds), including a `REPOS` folder you pointed somewhere else.
- Run the tools on your `PATH`, and use common toolchains (Node, Bun, Python,
  Rust, Go, Nix, mise, Homebrew) and their caches. They can also read the Node
  runtime and agent adapters Buzz installs, such as `claude-agent-acp`.
- Use their own state: `~/.claude`, `~/.codex`, Goose config, and the macOS
  keychain (for Claude Code and Codex logins). On macOS also `~/.cache` and
  `~/Library/Caches`.
- Read your Git configuration and use temporary directories.
- Reach the network. The sandbox limits files only.

Agents **cannot**:

- Read SSH and GPG keys, cloud and cluster credentials (`~/.aws`,
  `~/.config/gcloud`, `~/.kube`, `~/.docker`), registry and Git credentials
  (`~/.npmrc`, `~/.netrc`, `~/.git-credentials`), browser data, shell history,
  or shell startup files (`.zshrc` and friends).
- Read or write anything else in your home folder, such as `~/Documents`,
  `~/Desktop`, or other projects, unless you grant it.

Turn the sandbox off, or check why agents cannot start in it, under
**Settings → Agents → Agent sandbox**. Turning it off lets agents read and
change any file your account can.

## Giving agents more access

Buzz keeps two profiles in nono's profile folder (`~/.config/nono/profiles`,
or `$XDG_CONFIG_HOME/nono/profiles`):

| File | Owner |
|------|-------|
| `buzz-agent.json` | Buzz. Rewritten on every launch; do not edit. |
| `buzz-agent-local.json` | You. Created once, never overwritten. Agents run with this one. |

Dev builds use `buzz-dev-agent*.json`, and demo builds use
`buzz-demo-<name>-agent*.json`, so builds never share settings.

To grant access, click **Edit** in the Agent sandbox settings (or open
`buzz-agent-local.json` yourself) and add paths:

```json
{
  "meta": { "name": "buzz-agent-local", "version": "1", "description": "..." },
  "extends": "buzz-agent",
  "filesystem": {
    "read": ["$HOME/Documents/notes"],
    "allow": ["$HOME/code/my-project"]
  },
  "platform_overrides": {
    "macos": {
      "filesystem": {
        "allow": ["$HOME/Library/Keychains"],
        "bypass_protection": ["$HOME/Library/Keychains"]
      }
    }
  }
}
```

- `read` grants read-only access, `allow` read and write. Both cover folders
  recursively. Use `read_file` and `allow_file` for single files.
- Restart the agent to apply a change. Running agents keep the sandbox they
  started with.
- Your file can only **add** access. nono merges `extends` additively, so you
  cannot remove something `buzz-agent.json` grants.
- The `platform_overrides.macos` block is the keychain grant. If your agents
  log in with API keys rather than a Claude or Codex subscription, delete it.
- Other settings follow nono's profile format: run `nono profile guide`, or
  see [nono's profile docs](https://nono.sh/docs/cli/features/profiles-groups).

**Reset** in the settings restores the default file.

### Checking your profile

Buzz validates the profile before every agent start, and Settings shows the
error if it is invalid. To check it yourself with the nono bundled in Buzz
(macOS path shown; use your own nono on Linux):

```bash
NONO=/Applications/Buzz.app/Contents/MacOS/nono
$NONO profile validate buzz-agent-local
# Show what an agent would be allowed to access:
cd ~/.buzz && $NONO wrap --allow-cwd --dry-run --profile buzz-agent-local -- /usr/bin/true
```

## Troubleshooting

**An agent gets "Operation not permitted" or "permission denied".** The path is
outside the sandbox. Grant it in your local profile and restart the agent.

**A tool needs credentials the sandbox protects** (for example `kubectl`
reading `~/.kube`, or `docker` reading `~/.docker`). nono denies these even
when granted, unless you also lift the protection for that path:

```json
"filesystem": {
  "read": ["$HOME/.kube"],
  "bypass_protection": ["$HOME/.kube"]
}
```

This gives every agent those credentials; grant only what your agents need.

**A folder you granted is still denied.** nono only grants paths that exist
when the agent starts. Create the folder (or log in to the tool that creates
it, such as `codex login` for `~/.codex`), then restart the agent.

**Settings says agents can't start in the sandbox.** The message names the
reason:

- *nono is missing from this Buzz install*: reinstall Buzz. Builds from source
  get it from `just dev` and the other run recipes, or from
  `scripts/fetch-nono.sh`.
- *the profile is invalid*: fix the file the message names, or click Reset.
- *needs the Buzz nest directory*: Buzz could not create `~/.buzz`; check that
  your home folder is writable.

You can always turn the sandbox off in the same place.

**Codex on macOS.** macOS cannot run Codex's own sandbox inside nono, so Buzz
starts Codex in its "Full access" mode (`INITIAL_AGENT_MODE=agent-full-access`)
and nono is the boundary. Codex does not ask for approvals in that mode. One
Codex feature, the agent-file change report, still tries to use Codex's own
sandbox and fails on macOS. On Linux, Codex keeps its own sandbox as well.

**Claude Code's Bash sandbox.** If you enabled Claude Code's own sandbox in
its settings, it fails on macOS for the same reason. Turn it off for Buzz
agents; nono already sandboxes them.

**Windows.** nono supports Windows only through WSL2, so agents run without
the sandbox and the setting is hidden.

## For contributors

- **Spawn path:** `desktop/src-tauri/src/managed_agents/sandbox.rs` builds the
  `nono wrap --profile <nest>-agent-local -- buzz-acp` command and its
  per-launch grants. All local agents start through `spawn_agent_child`.
- **Base profile:** `managed_agents/sandbox_profile.json`, installed by
  `sandbox_profile.rs`. When a runtime needs a new path for every user, add it
  here rather than per spawn. Validate with `nono profile validate`.
- **Bumping nono:** edit `desktop/src-tauri/nono.lock` (version plus one
  SHA-256 per target, from the release's `SHA256SUMS.txt`), then run
  `scripts/fetch-nono.sh` for your target. Release builds fetch it through
  `scripts/bundle-sidecars.sh`.
- **Reserved env:** `NONO_*` and `BUZZ_ACP_OUTER_SANDBOX` cannot be set from
  agent, persona, or global env.
