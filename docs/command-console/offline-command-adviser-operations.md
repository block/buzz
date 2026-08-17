# Offline Command Adviser operations

This is the short operating procedure for taking the existing Command Adviser stack to sea. It does not introduce another service.

## What must be running

| Component | Accepted local identity | Check |
| --- | --- | --- |
| Command Adviser | Developer ID signed `/Applications/Command Adviser.app` | `codesign --verify --deep --strict` |
| Relay | `http://127.0.0.1:3000/health` | response is `ok` |
| Model | `gemma4-26b-official`, 65,536 context, reasoning off, parallelism one | real `GEMMA64 READY` generation |
| RAG | snapshot `f88174b38ae3bca3c0339d0d0bb9dafdec2fbb2507503c1b11e830c4895b735d` | ADF Doctrine semantic result with document, location, and `point_id` |
| Memory | `http://127.0.0.1:18006/mcp` | MCP initialize reports server `memory` |
| Skills | verified `~/.buzz/.agents/skills/learned-*` projection | both `SKILL.md` and `.skill-version.json` exist |

## Before sailing

1. While Internet and the home LAN are available, start LM Studio and load only one LLM, `gemma4-26b-official`, at the accepted settings. The `bge-m3-offline` embedding instance remains loaded for local RAG.
2. Refresh the Mac-local RAG snapshot through the Phase 2 process. Do not replace the active snapshot unless its semantic evaluation passes.
3. Back up the current Memory vault, Command Brief database, trusted-source configuration, and installed application. Encrypt state archives with `/Users/matthewwarren/.config/command-adviser/backup-encryption.key`; never copy the key into the bundle.
4. Build the manifest. Metadata-only mode inventories payloads in place and does not duplicate the 17 GB model. Use `--materialize` only with an explicit external destination that has enough space.
5. Run the readiness checker once online. `components_ready: true` with `ready: false` is expected while an external default route remains.
6. Disable Wi-Fi, Ethernet, Tailscale/VPN, and any other external route. Run the same checker again; only `ready: true` is the offline pass.

The standard readiness command is:

```bash
scripts/check-disconnected-readiness.sh \
  --manifest "/Users/matthewwarren/Command Adviser Backups/phase5-20260817/manifest.json" \
  --report "/Users/matthewwarren/Command Adviser Backups/phase5-20260817/readiness.json" \
  --rag-snapshot f88174b38ae3bca3c0339d0d0bb9dafdec2fbb2507503c1b11e830c4895b735d \
  --rag-collection "ADF Doctrine" \
  --rag-query "ADFP 5.0.1 Joint Military Appreciation Process" \
  --require-skills \
  --recovery-reserve-bytes 10737418240
```

## Normal disconnected operation

- Leave one LM Studio instance loaded. Adviser retrieval and preparation may overlap; generation remains FIFO at capacity one.
- Use Command Adviser normally. A failed cloud connector is not a reason to enable cloud fallback during the disconnected window.
- A source outage must appear as degraded. Do not describe a health-only result as doctrine retrieval.
- Memory and skill writes remain local and durable. Reconcile with home services only after connectivity is intentionally restored.

## Restart and recovery matrix

| Interrupted component | Expected symptom | Recovery | Required post-recovery proof |
| --- | --- | --- | --- |
| Command Adviser | UI and managed agents stop; local services remain | reopen the installed app | one desktop process, nine ACP processes, one Apple wake watcher, readiness rerun |
| LM Studio | model check fails; no cloud fallback | reopen LM Studio/server and load the accepted LLM instance | `scripts/check-offline-model.sh` passes; exactly one loaded LLM plus the accepted RAG embedding instance |
| RAG retrieval | RAG semantic check fails | restart the loopback retrieval process and Qdrant container | fixed ADF Doctrine query returns the same snapshot ID and a `point_id` |
| Memory MCP | Memory initialize fails; capture queues locally | If the service is loaded, run `launchctl kickstart -k gui/$(id -u)/com.navigatorran.command-adviser-memory`. If `launchctl print` says it is not found, first run `launchctl bootstrap gui/$(id -u) "$HOME/Library/LaunchAgents/com.navigatorran.command-adviser-memory.plist"`, then kickstart. | MCP initialize reports `memory`; queued projection drains |
| Relay | agents and publication go offline | `launchctl kickstart -k gui/$(id -u)/xyz.block.command-adviser.relay` | relay health is `ok`; queued brief republishes the same event ID |
| Mac | all processes stop | log in, start LM Studio if it is not configured for startup, open Command Adviser | full readiness check, exact model identity, snapshot identity, Memory, skills, pending publication |

A single macOS Keychain approval after installing a newly signed build is acceptable. Repeated prompts on subsequent controlled restarts are a product failure; stop and use the retained application rollback rather than repeatedly approving them.

## Eight-hour soak

Create a small executable wrapper containing the readiness command above, then run:

```bash
python3 scripts/monitor-disconnected-soak.py \
  --probe-program "/absolute/path/to/readiness-wrapper.sh" \
  --audit-db "$HOME/.buzz/command-brief/audit.db" \
  --cloud-log "$HOME/Library/Application Support/xyz.block.buzz.app/agents/logs/AGENT.log" \
  --monitor-dir "$HOME/.buzz" \
  --monitor-dir "$HOME/Library/Application Support/Command Adviser/Memory" \
  --report "/Users/matthewwarren/Command Adviser Backups/phase5-20260817/soak.json"
```

Pass every active agent log with another `--cloud-log`. The monitor runs for eight hours by default and fails on a cloud-attempt marker, prolonged component loss, a newly stuck scheduled brief, duplicate publication for one run, or excessive disk growth. It writes every sample atomically; rerun with `--resume` after an interruption.

## Rollback and stop conditions

Stop acceptance and restore the retained app if any of these recur after one documented recovery attempt:

- the qualified model cannot generate or a second LLM instance loads;
- the semantic RAG canary cannot return citation identity;
- Memory cannot resume queued local projection;
- one Command Brief run publishes more than one terminal event;
- the queue remains active beyond 45 minutes without progress;
- a cloud attempt occurs while disconnected acceptance is active;
- repeated Keychain prompts appear after the first approved launch; or
- free space falls below the 20% recovery reserve.

RAG rollback changes the active pointer only to a previously verified snapshot and then reruns the semantic canary. Skill rollback uses the existing immutable parent version and active pointer; it never deletes the rejected version. Post-deployment reconciliation is performed only after the owner restores connectivity and first retains the disconnected acceptance report and local backups.
