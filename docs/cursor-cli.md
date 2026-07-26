# Cursor CLI in Buzz

Buzz can use Cursor's own CLI as a native ACP harness when `agent` (or the
backward-compatible `cursor-agent`) is available on the desktop PATH.

Install Cursor CLI with Cursor's official installer, then sign in through the
vendor CLI:

```sh
curl https://cursor.com/install -fsS | bash
agent login
```

Buzz discovers the command, checks `agent status`, and offers the existing
managed-agent sign-in flow. Buzz does not store Cursor OAuth tokens or API
keys. If Cursor advertises ACP authentication methods, Buzz uses them;
otherwise it opens the visible vendor login command.

Cursor is launched through its native ACP entrypoint (`agent acp` or
`cursor-agent acp`). Available models are discovered from ACP when the CLI
advertises them. A selected model is passed at process startup with Cursor's
`--model` flag; changing it affects the next launch, not an already-running
session.

On Windows, use the Cursor CLI from WSL. Buzz does not provide a native
Windows Cursor installer or promise native Windows execution.
