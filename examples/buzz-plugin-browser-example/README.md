# Buzz browser plugin example

A minimal, standalone reference implementation of the Buzz browser plugin
contract `0.1.0-alpha`. It shows what an external plugin author needs to
build, package, and install a `browser` contribution — nothing else.

This crate has no dependency on Buzz source or any `buzz-*` crate: it is a
self-contained Cargo package (its own `[workspace]`, `serde_json` only) that
you could copy out of this repository and build on its own.

## Alpha / v1 compatibility

This is an alpha prototype (`0.1.0-alpha`), not the version-1 plugin system.
It is not forward-compatible with that unbuilt system. See
[`docs/plugin-browser-prototype.md`](../../docs/plugin-browser-prototype.md)
for the published contract this example implements.

## Trust model

Your plugin executable is trusted code. It runs as the same operating-system
user as Buzz, in its own process group, with process separation providing no
security boundary. Buzz discloses this to the user in the install consent
dialog. Do not treat the plugin process boundary as a sandbox.

The remote content webview is separate from your process and has no IPC
bridge back to Buzz — see the contract's "What the host does with your
answer" section for what that surface does and does not expose.

## Platform support

The browser surface only opens on macOS. `runtime.targets` may list other
target triples, but the surface will not open there. Installing still
requires a `runtime.targets` entry that matches the host's exact target
triple — a package built only for `x86_64-apple-darwin` cannot install on
an `aarch64-apple-darwin` host, and vice versa.

## Build

```sh
python3 build.py --output <package-directory> [--home-url <http-or-https-url>]
```

- `--output` — where to write the installable package (created if missing;
  its `bin/` contents are replaced on every run, so re-running with the same
  `--output` never leaves a stale binary or manifest field behind).
- `--home-url` — the URL the plugin resolves for `kind: "home"`, baked into
  the binary at compile time via `BUZZ_BROWSER_HOME_URL`. Defaults to
  `https://example.com/`. Buzz supplies no runtime home-URL environment
  variable — this flag is purely a convenience of this example's build
  tooling. A local development fixture can pass
  `--home-url http://127.0.0.1:<port>/home` without changing the host or wire
  API. An invalid value (missing/unsupported scheme, missing host, or over
  2048 characters) exits non-zero before anything is built or written.

The script determines the host's actual Rust target triple (`rustc -vV`),
builds the crate in release mode, copies the resulting executable into
`bin/<target-triple>` inside the output directory, and writes `manifest.json`
with the executable's real SHA-256 and byte count.

Install the generated **output directory**, never this source directory and
never `target/`. From Buzz: Settings → Plugins → install, pointing at the
output directory.

The manifest and `bin/` checked into this source directory are illustrative
placeholders only (fixed zero-filled SHA-256, zero byte count) — they exist
to show the on-disk layout, not to be installed directly.

## Installing twice

An already-installed plugin `id` is rejected on a second install attempt,
including when the existing installation is disabled — the existing
package, grant, and session are left untouched. Uninstall the existing
plugin first if you want to reinstall the same `id` (for example, after
rebuilding with a different `--home-url`).

## Wire protocol

Newline-delimited JSON-RPC 2.0 on stdin/stdout: exactly one JSON object per
line, UTF-8, no embedded newlines. Only `server/discover`, `tools/list`, and
`tools/call` (for the single `browser.resolve` tool) are ever called;
`notifications/*` sent by the plugin are ignored. Deadlines are 5 s for
`server/discover` and `tools/list`, 2 s for `tools/call` — a missed deadline
cancels the request and any later response for it is discarded.

stdout carries JSON-RPC only. Write diagnostics to stderr — the host keeps
the last 8 KiB per session in the Buzz application log, but stderr never
reaches the user interface; the surface only ever shows a fixed sentence per
error code.

## Lifecycle

| Event | What your process sees |
|---|---|
| Install | nothing — your process is not started |
| Surface opened | spawned, then `server/discover`, `tools/list`, then a `"home"` call |
| Address submitted | an `"address"` call; any in-flight call is cancelled first |
| Surface closed, disable, uninstall, community switch | stdin closes, then `SIGTERM` to the process group, then `SIGKILL` after 2 s |
| Disable | package and grant stay on disk; enable restarts you without reinstalling |
| Uninstall | package directory and grant are deleted |

Exiting non-zero or crashing mid-call surfaces as `plugin-unavailable`; the
rest of Buzz is unaffected, and Buzz restarts your process the next time the
surface opens (not automatically).

## Resolving addresses

`browser.resolve` is called with `kind: "home"` (no `input`) when the surface
opens, and `kind: "address"` with the user's typed `input` when they submit
the address field. Return a `resolved` outcome with a `url`, or a `rejected`
outcome with a `reason` — a `tools/call` result, not a JSON-RPC error. Use a
JSON-RPC error only for an unknown tool name or a malformed request.

This example's `address` handling: trims the input, accepts it unchanged if
it already starts with `http://` or `https://`, adds `https://` to
otherwise-bare host-like input (contains a `.`, doesn't start or end with
one), and rejects empty input, input over 2048 characters, input containing
whitespace, and input with any other scheme (anything containing `://` that
isn't `http://`/`https://`) or that doesn't look like a host.

## What the host does with your `url`

Every `url` you return is validated before loading, with the same rules
applied to in-page link navigations: scheme must be `http`/`https`
(`about:`, `file:`, `data:`, `javascript:`, `tauri:`, `buzz-media:`, and
`asset:` are refused); the parsed hostname must not be `ipc.localhost`,
`buzz-media.localhost`, or `tauri.localhost` at any port, nor `localhost` on
port `1420`; length at most 2048 characters; and it must parse as an
absolute URL. A refusal surfaces as `navigation-denied`. Everything else on
the open web loads, though the load itself can still fail independently
(`load-failed`, `plugin-timeout`).

## Manifest

One `browser` contribution, exactly the `browser.browse` grant, strict
schema (an unrecognized key at any level fails install). See
`manifest.json` for the field layout and
[`docs/plugin-browser-prototype.md`](../../docs/plugin-browser-prototype.md)
for the full field reference.
