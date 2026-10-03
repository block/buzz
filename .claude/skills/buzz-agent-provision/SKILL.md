---
name: buzz-agent-provision
description: >
  Give a non-Claude-Code agent — goose, codex, claude-agent-acp, hermes, anything
  buzz-acp runs — an identity on the relay, and print the env block to deploy it
  with. Mints a keypair, enrols it, publishes its name, and puts it in a channel,
  because buzz-acp does none of that and an agent missing any of it boots and
  sits idle. Use when hosting an agent, not for a Claude Code session.
version: 1
---

# Provision an agent identity

```bash
# project install (this repo), from the repo root:
.claude/skills/buzz-multi-session/scripts/buzz-agent-provision.sh <name> \
  [--channel <name>] [--command <harness>] [--owner <pubkey>] [--auth-tag <json>]

# user install, from anywhere:
~/.claude/skills/buzz-multi-session/scripts/buzz-agent-provision.sh <name> ...
```

`buzz-acp` never claims an invite, never publishes a profile and never joins a
channel. It assumes all of it, and given none of it boots to `no channel
subscriptions resolved — agent will sit idle`. This is that gap, closed in one
command, ending in the env block for a Dockerfile, a fly secret or a systemd
unit.

**The private key is never printed** — only its path, and two ways to load it
that keep it out of a terminal, a log, a shell history and `ps`.

The identity is deliberately **not** bound to the session that created it, so a
later `/rename` cannot rename a running daemon's key out from under it. No
watcher is armed: the harness is its own event loop.

**Ownership is the part that cannot be automated.** An unowned agent is not a
formality: buzz-acp's `--respond-to` defaults to `owner-only`, so it connects and
ignores everyone. `--auth-tag` takes a real NIP-OA attestation, which only the
owner's secret key can mint; `--owner` records the pubkey and says plainly that
it is not the same thing. Both paths print the full cost rather than leaving it
to be discovered. Note that a key which **enrols itself can never have an owner
recorded** — the relay writes the owner only for a key admitted through its
owner — so provisioning with `--auth-tag` deliberately does not claim an invite.

This skill is one entry point to the shared scripts and adds no behaviour of its
own. The reasoning — the harness identity table, the ownership gap, why this is
not mirrored to other runtimes — is documented once, in the
**`buzz-multi-session`** skill.
