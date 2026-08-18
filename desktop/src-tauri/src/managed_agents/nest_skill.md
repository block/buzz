---
name: buzz-cli
description: >
  Buzz CLI for relay operations: owner-reviewed agent drafts, messaging,
  channels, DMs, users, workflows, feed, reactions, canvas, social, repos,
  uploads, and agent memory.
version: 1
---

# Buzz CLI Skill

## Environment

`BUZZ_PRIVATE_KEY` is set by the harness at runtime or by the developer's environment. If missing, tell the user to set it (hex or nsec format). Never read or echo the value.

`BUZZ_RELAY_URL` defaults to `http://localhost:3000`. In development, the user may need to set this to a staging or production relay URL.

`BUZZ_AUTH_TAG` is required for `buzz agents draft-create` and `buzz agents draft-update` because those commands send owner-reviewed Desktop drafts. If missing, explain that this managed agent cannot open owner-reviewed agent drafts from chat.

Run the bundled CLI with `--help` and `<command> <subcommand> --help` to discover all flags, arguments, and usage. This skill documents only what `--help` cannot tell you.

## Conversational Agent Management

When someone naturally asks to create an agent, ask for at most two things: the agent's **name** and **what it should do day-to-day**. Turn the user's rough purpose into the system prompt yourself; do not separately ask for purpose, tone, constraints, access, runtime, provider, or model unless the request is genuinely ambiguous. Then run:

```bash
buzz agents draft-create \
  --channel <current-channel-uuid> \
  --display-name "Research helper" \
  --system-prompt "Find reliable sources and summarize them concisely."
```

Use the UUID from the current Buzz `[Context]`; do not ask the user for it. Do not ask about runtime, provider, model, credentials, environment variables, or access. Desktop uses the machine's real defaults, and new agents start as **Only me**. The command sends an encrypted draft to the owner's Desktop. It does not create the agent until the owner reviews and saves the form, so report the result as “ready for review,” never “created.”

For an explicit change to an existing personal agent, use:

```bash
buzz agents draft-update --channel <uuid> --agent-name "Current name" \
  --system-prompt "Updated instructions"
```

Run `buzz agents draft-update --help` for optional runtime, provider, model, rename, and access changes. Prefer these CLI commands over any legacy MCP agent-management tools.

When the owner explicitly asks to install, set up, configure, reconfigure, regenerate, expand, enable, review, or protect this Agent with Nxtlinq, completing the request means sending a review draft in the **same turn**. Inspect only ordinary documentation/source that the current policy already permits, construct the narrowest useful policy, and then invoke the structured `nxtlinq_setup` tool; do not stop after describing the policy, print policy JSON as the final result, offer to send it later, or ask for another confirmation. The Desktop review is the owner's confirmation boundary and the tool itself installs or changes nothing. Do not invoke `buzz agents nxtlinq-setup`, its `--help`, or another setup reference command through shell; the structured tool is pinned to this Buzz build and cannot be shadowed by an older CLI on PATH.

Call the structured tool with this exact envelope shape (values below are examples). `name`, `version`, `scope`, `aud`, and `capabilities` belong **inside `policy`**; never put them at the top level and never omit `policy`:

```json
{
  "channel": "<current-channel-uuid>",
  "owner_project_root": "/absolute/path/supplied/by/owner",
  "explanation": "Read ordinary project documentation and source; exclude secrets and signing material.",
  "policy": {
    "name": "customer-project-agent",
    "version": "1.0.0",
    "scope": ["demo:structured-capabilities"],
    "aud": ["nxtlinq-authorization-gateway"],
    "capabilities": [
      {
        "type": "filesystem:read",
        "include": ["README.md", "package.json", "src/**"],
        "exclude": [".env*", "**/.env*", "nxtlinq/**"]
      },
      {
        "type": "mcp:connect",
        "servers": ["buzz-dev-mcp"]
      }
    ]
  }
}
```

An Agent that is already protected may be unable to inspect files or run commands needed for the newly requested work. Inspection is optional evidence, never a prerequisite for opening review. That denial is expected and is **not** a setup blocker: never retry through shell, ask the owner for a narrower target merely because inspection was denied, or refuse to open review. If any preliminary file, MCP, shell, or help lookup is denied, proceed immediately to the structured tool in the same turn. Use the owner's stated task, the files already accessible, and the denied operation itself to propose the smallest additional capability. For regeneration, the current owner-reviewed proposal and owner guidance supplied in the request are sufficient input; file inspection is not required. If the request is a generic setup/review with no more specific task evidence, submit the conservative normal Buzz baseline described below. Desktop shows the existing manifest beside the proposal and lets the owner edit it before anything is applied. The trusted `nxtlinq_setup` control-plane tool remains available specifically so an existing policy can be reviewed or expanded without first granting the Agent permission to read its own manifest.

Use the current Buzz `[Context]` channel UUID as `channel`. `owner_project_root` is mandatory and must copy the absolute project path explicitly supplied by the owner in the current request exactly; never replace it with the shell/MCP working directory, the Agent's configured workspace, `~/.buzz-dev/REPOS`, or any inferred default. If the owner did not supply an absolute path, ask for the exact project path and do not submit a draft yet. An existing real project directory does **not** need to be initialized for Nxtlinq before submission: Desktop review owns the explicit Attest initialization and secure-key ceremony. Never ask the owner to run `nxtlinq-attest init` merely because `nxtlinq/` or its manifest is absent.

The project root is absolute, but every filesystem `include` and `exclude` in the manifest is **relative to that root**. Never prefix a policy glob with `owner_project_root`. Use exactly `scope: [demo:structured-capabilities]` and `aud: [nxtlinq-authorization-gateway]`; do not put paths or capability names in `scope`, because legacy scope entries can authorize an entire operation family. A normal Buzz Agent draft starts with a narrow `filesystem:read` (for example `README.md`, `package.json`, and `src/**`), the complete sensitive excludes supplied by the `nxtlinq_setup` schema/normalizer (environment files, npm/netrc/PyPI credentials, Git/Nxtlinq metadata, AWS/Docker/SSH credentials, and key material), plus `mcp:connect` with `servers: [buzz-dev-mcp]` so the Gateway can establish the required bundled MCP connection. Connection authority is not invocation authority.

Default-deny filesystem writes, terminal execution, and `mcp:invoke`; add one only when the owner's intended future work actually needs it. A `filesystem:write` grant does not imply read; editing with `str_replace` needs matching read and write coverage for the target. A terminal grant uses exact raw shell tool strings such as `git status` or `npm start`, without adding `bash -lc`, `pwd`, `cd`, preflight commands, or absolute workspace paths. Terminal commands are independent of filesystem excludes and can read files themselves, so do not add shell merely to make file access convenient. `environment` contains variable names only, must include `PATH`, and names every additional variable the command may receive; never include values or `NAME=value`, and never request host identity variables such as `BUZZ_PRIVATE_KEY`, `NOSTR_PRIVATE_KEY`, or `BUZZ_AUTH_TAG`. Do not put bundled `read_file`, `str_replace`, or `shell` under `mcp:invoke`; Buzz maps them to filesystem or terminal decisions. Local `view_image` uses filesystem read, while fetching a remote image requires the explicit `mcp:invoke` selector `servers: [buzz-dev-mcp]`, `tools: [view_image]`. Other external MCP tools require both an explicit `mcp:connect` server and a narrow `mcp:invoke` selector with exactly one server (and only its required tools), preventing accidental server/tool cross-products.

After success, report that the draft is awaiting owner review and include the returned request ID. Only stop without sending a draft if a concrete setup prerequisite fails (for example, no host-bound channel context, the supplied project path does not identify a real directory, or the structured tool rejects the draft); missing Attest initialization and an ordinary filesystem, terminal, or MCP authorization denial are not such prerequisites. Report only the exact blocker returned by the setup tool.

Propose policy fields only; do not inspect `nxtlinq/`, dotfiles, credentials, or other secrets. Never install the Gateway, directly edit the manifest, or request/read/pass a signing private key. Desktop owns installation and invokes the reviewed standard Attest init when needed; Attest generates the identity, after which Desktop relocates the private key into secure storage before installing public project state. Desktop also owns diff application and owner-initiated native signing. The Agent never participates in key generation, selection, storage, or signing.

For capabilities, use only the Gateway manifest fields: filesystem `include`/`exclude`; terminal `commands`/`environment`; canonical MCP `servers` and `tools`; optional `approvalRequired`. Every selector is a string array. Never invent constraints such as `command`, `args`, `cwd`, or `network`. Conversational setup rejects `approvalRequired: true` because Buzz does not yet provide that interactive approval flow; omit it (or use `false`).

## Git Repositories

Buzz hosts real git repos, and **you can own one yourself** — no human key needed. `repos create` signs the announcement with *your* key, so the repo is owned by whoever runs it; the owner segment in the clone URL is your own pubkey (hex, not a username). Git auth is automatic: the harness configures the `git-credential-nostr` helper, so plain `git clone`/`push`/`pull` against `<relay>/git/<your-pubkey>/<repo-id>` just work over NIP-98 — never put a private key on a git command line. Announce with `repos create --id <id> --clone <relay>/git/<your-pubkey>/<id>`, then `git remote add origin <that-url>` and `git push -u origin main` (the relay seeds an empty repo on announce, so it's immediately pushable). Requires git 2.46+ for the credential protocol.

Manage your repository's enforced branch and tag rules with `repos protect list|set|remove`. Ref patterns must use full Git names such as `refs/heads/main` or `refs/tags/*`; supported rules are `--push owner|admin|member`, `--no-force-push`, `--no-delete`, and `--require-patch`. `protect set` replaces the complete rule for that exact pattern, so omitted constraints are removed. Protection updates preserve every unrelated metadata tag and return exit code 5 when a newer NIP-33 head wins a concurrent write.

## Output Contracts

Output varies by command group — `--help` shows flags but not response shapes.

**Read commands** (messages, channels, users, feed, workflows): normalized JSON arrays with `sig` stripped. Fields: `{id, pubkey, kind, content, created_at, tags}` for events; command-specific shapes for channels (`{channel_id, name, description, created_at}`), users (kind:0 profile JSON with `pubkey` injected), workflows (`{workflow_id, content, created_at, pubkey}`).

**Write commands**: all return `{event_id, accepted, message}`. Create commands add the generated entity ID: `channels create` → `channel_id`, `dms open` → `dm_id`, `workflows create` → `workflow_id`. Agent draft commands add `{request_id, action, saved: false}` because they only open an owner-reviewed Desktop draft.

**Exceptions to the above patterns:**

| Command | Output |
|---------|--------|
| `canvas get` | raw markdown string or `null` — NOT a JSON envelope |
| `social *`, `repos get/list` | raw Nostr event JSON INCLUDING `sig` — different contract than read commands above |
| `repos protect list` | `{repo_id, protections: [{ref, rules}], unknown_rules, validation_error}` |
| `upload file` | pretty-printed multi-line `BlobDescriptor`: `{url, sha256, size, type, uploaded}` |
| `mem get` | raw bytes to stdout, no trailing newline |
| `mem hash` | SHA-256 hex string |
| `mem set/patch/rm` | nothing to stdout; progress to stderr |
| `mem ls` | tab-delimited (`slug\tcreated_at\tevent_id`) by default; `--json` for JSON array |
| `reactions get` | `{"reactions": [{emoji, count, pubkeys}]}` — aggregated, not raw events |
| `pack validate/inspect` | human-readable text, not JSON |

**Errors** go to stderr as `{"error": "<category>", "message": "<detail>"}`. Exit codes: 0 = success, 1 = input/not-found, 2 = relay/network, 3 = auth, 4 = other, 5 = write conflict (value superseded).

## Compact Format

`--format compact` is a global flag — position it before the subcommand:

```bash
buzz --format compact channels list          # [{channel_id, name}]
buzz --format compact messages get --channel <UUID>  # [{id, content, created_at}]
buzz --format compact users get              # [{pubkey, display_name}]
buzz --format compact feed get               # [{id, content, created_at}]
```

Write commands are unaffected. `--format json` (default) returns full fields.

## Communication Patterns

**Mentions that notify:** Keep readable `@Name` text in message content and, when intended pubkeys are known, pass the identities in the same send with repeatable `--mention <hex-or-npub>`. Any explicit identity (`--mention` or `nostr:npub...`) permits unresolved or ambiguous `@Name` text as presentation-only; uniquely resolved member names still add recipients. Include a pubkey for every presentation-only name that should notify. The CLI reports the signed event's `mention_pubkeys`; no follow-up verification command is needed. Without explicit identities, names resolve against current channel members. An unresolved/ambiguous name or non-member target stops before publishing. Add membership separately only when authorized, then retry; sending never changes membership automatically.

```bash
buzz messages send --channel <UUID> \
  --content "@Alice check this" --mention <alice-pubkey>
```

## DM Management

`dms hide --channel <UUID>` hides a DM from the agent's DM list. Restore by re-opening with `dms open --pubkey <hex>`.

## Channel Policies

`channels set-add-policy --policy <value>` controls who can add you to channels:
- `anyone` (default) — any authenticated user can add you to open channels
- `owner_only` — only your provisioned owner can add you
- `nobody` — no one can add you; self-join via `channels join`

## Workflow Inputs

`workflows trigger --workflow <UUID> --inputs '<json>'` passes input variables as the trigger event's content. Omit `--inputs` for parameterless workflows.

## Feed Filtering

`feed get --types <comma-separated>` filters by category. Valid types: `mentions`, `needs_action`, `activity`, `agent_activity`. Omit for all categories.

## Pagination

`messages thread --depth-limit <n>` caps reply nesting depth (relay extension hint — may be ignored).

`social notes --before-id <hex64>` enables composite cursor pagination. Use with `--before <timestamp>` to avoid skipping same-second events.

## Gotchas

1. **`feed get` sorts newest-first** — every other list command sorts oldest-first. Don't assume consistent sort order.
2. **`users set-presence` is broken** — sends ephemeral kind:20001 via HTTP POST; relay rejects ephemeral kinds over HTTP. Will fail until WebSocket support is added.
3. **`workflow runs` always returns `[]`** — run history lives in the relay's database, not as Nostr events.
4. **`dms open` returns `dm_id`** — use this value as `--channel` for subsequent `messages send/get` commands on that DM.
5. **Content max 65,536 bytes** (exit 1 if exceeded). Diffs auto-truncate at 61,440 bytes at a hunk boundary.
6. **`users get` always returns an array** — even for a single pubkey lookup. Never expect a bare object.
7. **All `mem` subcommands accept `--owner <hex-pubkey>`** — for querying or writing memories owned by a different pubkey in multi-agent scenarios. Defaults to the owner from `BUZZ_AUTH_TAG`.
8. **`mem rm` cannot delete `core`** — use `mem set core ''` instead.

## Forum Posts

`messages send --kind` routes to different event builders:

- Omitted or `9` → stream message (default)
- `45001` → forum post (thread root)
- `45003` → forum comment (requires `--reply-to <event-id>`)

Other kind values are rejected. Use `messages vote --event <id> --direction up|down` to vote on forum posts.

## Message Formatting

Message content is rendered as GitHub-flavored Markdown on both desktop and mobile. Key formatting:

- **Fenced code blocks**: triple-backtick with a language tag for syntax highlighting (190+ languages supported). Omitting the language tag renders a styled monochrome block.
- **Inline code**: single backticks for inline monospace.
- **Mentions**: plain `@name` — do NOT bold or italicize (formatting prevents alert delivery).
- **Links, images, tables, blockquotes, headings**: standard GFM.

## Mem Patch Workflow

For safe concurrent writes, use hash-based conflict detection:

```bash
HASH=$(buzz mem hash <slug>)                                    # 1. get current SHA-256
# ... build unified diff ...
buzz mem patch <slug> --base-hash "$HASH" --patch-file diff.patch  # 2. apply with check
```

Exit code 5 if the value changed since the hash was read (another agent wrote first). Retry by re-reading, re-diffing, and re-patching.

Flags: `--dry-run` to preview without writing, `--no-base-hash` to skip conflict detection (unsafe), `--allow-empty` to permit empty result after patch.

## Polling Pattern

The relay has no push or webhook support. Poll with a `--since` cursor:

1. `buzz messages get --channel <UUID> --limit 50` — note the maximum `created_at` from results
2. Sleep 10-30 seconds
3. `buzz messages get --channel <UUID> --since <max_created_at> --limit 50`
4. Repeat, advancing `--since` each iteration

Minimum interval: 5 seconds (relay rate limiting). Use 10s for low-latency, 30s for background monitoring. `feed get` always returns newest-first regardless of `--since`.
