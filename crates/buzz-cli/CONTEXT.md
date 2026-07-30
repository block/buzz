# Managed context CLI

`buzz context` is the versioned command surface for sovereign journals,
replication, handoffs, and content-addressed artifact custody. The
`scripts/buzz-ctx` executable is only a compatibility launcher for historical
spellings; it contains no host paths, credential selection, relay policy, or
lifecycle logic.

## Profiles and XDG layout

`BUZZ_PROFILE` selects a profile and defaults to `default`.

| Purpose | Default location |
| --- | --- |
| Profile | `${XDG_CONFIG_HOME:-$HOME/.config}/buzz/profiles/<profile>.toml` |
| Durable state | `${XDG_DATA_HOME:-$HOME/.local/share}/buzz-local-relay/<profile>/` |
| Journal | `<data_root>/sovereign.ndjson` |
| Artifact cache | `<data_root>/artifacts/` |
| Replication cursors | `<data_root>/cursors/` |

`BUZZ_CTX_HOME` is an explicit compatibility override. When set, the profile
is read from `<BUZZ_CTX_HOME>/profile.toml`, or a solo-compatible profile is
resolved in place when that file is absent. `~/.buzz-local` is detected for
migration but is never silently selected as the managed default.

Profile schema 1:

```toml
schema_version = 1
data_root = "/srv/buzz/context/vumc"

[node]
label = "vamc3w36217hk"

[relays]
local = "http://127.0.0.1:7777"
rendezvous = "https://rendezvous.example"

[paths]
journal = "sovereign.ndjson"
artifacts = "artifacts"
cursors = "cursors"

[context]
default_h = "shared/tooling"
streams = ["shared/tooling", "shared/steward-reports"]

[replication]
source = "vamc3w36217hk/sovereign"
cursor_file = "cursors/vamc3w36217hk-sovereign.push-cursor"
streams_file = "streams.json"
streams = ["shared/tooling", "shared/steward-reports"]

[identities.journal_author]
provider = "file"
reference = "/run/credentials/buzz/journal.key"
label = "journal"

[identities.replication_transport]
provider = "file"
reference = "/run/credentials/buzz/transport.key"
label = "transport"

[identities.relay_witness]
provider = "public_key"
reference = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"

[identities.agent]
provider = "environment"
reference = "BUZZ_AGENT_SIGNING_KEY"
label = "claude-code"
auth_tag = "/run/credentials/buzz/claude-code.auth"

[identities.steward]
provider = "environment"
reference = "BUZZ_STEWARD_SIGNING_KEY"
label = "steward"
auth_tag = "/run/credentials/buzz/steward.auth"

[identities.artifact_source_reader]
provider = "environment"
reference = "BUZZ_RENDEZVOUS_READER_KEY"

[identities.artifact_destination_owner]
provider = "file"
reference = "/run/credentials/buzz/artifact-owner.key"

[identities.artifact_rendezvous_uploader]
provider = "environment"
reference = "BUZZ_RENDEZVOUS_UPLOADER_KEY"

[runtime]
handoff_reducer = "/opt/buzz/releases/0.1.0/bin/buzz-handoff-state"
relay_push = "/opt/buzz/releases/0.1.0/bin/buzz-relay-push"

[installation]
release_manifest = "/opt/buzz/releases/0.1.0/release.json"
```

Credential fields are provider references, never embedded private keys.
`provider = "file"` names a private file, `environment` names an environment
variable, and `public_key` is verification-only. An `auth_tag` path may attach
a NIP-OA owner attestation to an application role.

Application identities (`journal_author`, `agent`, `steward`), replication
transport, relay witness, and artifact custody identities are deliberately
separate. An enterprise/VUMC profile should keep them split. A one-operator
laptop may point several roles at the same credential; `doctor` reports that
collapse explicitly, so compatibility is visible instead of implicit.

Artifact synchronization always reads the rendezvous with
`artifact_source_reader` and writes the sovereign store with
`artifact_destination_owner`. `artifact_rendezvous_uploader` is used only when
announcing new bytes. A successful second sync reports zero fetched objects.

## Migration and rollback

Migration is dry-run first:

```bash
buzz context migrate \
  --rendezvous https://rendezvous.example \
  --context shared/tooling
buzz context migrate \
  --rendezvous https://rendezvous.example \
  --context shared/tooling \
  --apply
```

The command refuses mixed legacy/managed state and existing profiles that
point elsewhere. Apply creates an owner-only XDG profile atomically. It does
not move journals, artifacts, cursors, or credentials; the generated profile
references the legacy state in place. It is therefore restart-safe and
rollback means stopping the managed process and removing the new profile file.
The legacy bytes remain untouched. If the supplied legacy root is already the
canonical XDG data root, migration creates only the reference profile; that is
an in-place state layout, not a mixed-state conflict. Review the dry-run report
before removing any profile.

`BUZZ_CTX_HOME=/path/to/layout buzz context doctor --offline` is the explicit
no-migration compatibility path.

## Install and update

Install each build into an immutable, versioned directory and atomically move
the operator-controlled `current` symlink after validation. Do not overwrite a
running executable. A release manifest is JSON:

```json
{
  "version": "0.1.0",
  "git_revision": "40-lowercase-hex-commit",
  "sha256": "64-lowercase-hex-of-the-buzz-executable"
}
```

Point `installation.release_manifest` at that file. `buzz context version`
reports its embedded build revision, and `buzz context doctor` hashes the
running executable and reports manifest drift. Rollback moves the `current`
symlink to the prior immutable release and restarts the service.

Host B verification must run `doctor` against the installed executable without
modifying it, then run artifact sync twice and observe zero fetches on pass
two.

## Relay state versus configuration

The NDJSON journal, artifact bytes, and cursor files are mutable relay state.
The TOML profile, credential provider references, declaration heads, versioned
runtime paths, and release manifest are configuration/governance. Updating a
profile does not rewrite relay state; moving relay state requires a separately
reviewed operational procedure.

`replication.cursor_file` and `replication.streams_file` are resolved relative
to `data_root` and passed unchanged to `buzz-relay-push`. This keeps interactive
`buzz context sync` on the same checkpoint and stream selection as scheduled
replication. Older profiles infer `<journal>.push-cursor` and an existing
`<data_root>/streams.json`; set both fields explicitly for managed
installations. A profile that lists `replication.streams` without a streams
file fails closed instead of silently widening to a whole-journal mirror.

Session residue is context-bound. `buzz context log <project> <message>` uses
`context.default_h`; `--context <h>` overrides it. The command refuses to
write when neither boundary is available.

Useful checks:

```bash
buzz context version --json
buzz context doctor --json
buzz context artifact sync --dry-run --json
buzz context artifact sync --json
buzz context artifact sync --json
buzz context handoff acknowledge-invalid <open-event-id>...
```

`handoff acknowledge-invalid` is an archival operation for exact invalid legacy
opens. Every target must already reduce to `INVALID`, share one `h` context and
effective owner, and be authorized by the selected application identity. The
command signs and reduces the complete candidate batch before publication.
Successful acknowledgment reports `ACKNOWLEDGED_INVALID`; it never validates,
returns, or closes the original lifecycle.
