# Testing buzz-acp

Activate Hermit from the repository root, then run the complete package suite:

```sh
. ./bin/activate-hermit
cargo test -p buzz-acp
```

## Native Pi prompt transport

The pool tests exercise the production session composer, native transport setup,
legacy-fallback decision, and session invalidation. The executable tests check
new-session and restore argument selection, missing snapshots, and terminal login.

With Pi installed on PATH, also run:

```sh
cargo test -p buzz-acp --test pi_native_launcher -- --ignored
```

This starts the real Pi CLI through the built Buzz launcher and exports its live
system prompt over RPC. It checks exactly one framed base, profile, and core-memory
section. It uses a synthetic transcript and isolated Pi settings, disables extensions
and workspace context files, makes no model calls, and deletes its temporary files.
It does not change a running agent or its registration.

Do not reconstruct a production session's system prompt by reopening its transcript
with a profile-only `--system-prompt` override. Pi's HTML export reports the exporting
process's current system prompt, not a historical system prompt from the transcript.

Prompt snapshots live for a Buzz session. Subprocess restoration reuses that snapshot;
new Buzz sessions fetch and compose fresh standing context. Retiring a session removes
its snapshot. Existing transcript content is not rewritten by this transport.
