# Browser plugin prototype (`0.1.0-alpha`)

This file is the complete public contract. An external author needs no Buzz
source, Buzz crate, or software development kit (SDK). This alpha contract is
not compatible with the proposed [version-1 plugin system](plugin-system.md).

## Quick start

The browser surface runs on macOS. The plugin executable runs as your operating
system user, so install packages only from publishers you trust.

1. Build the example package:

   ```bash
   cd examples/buzz-plugin-browser-example
   python3 build.py --output /tmp/example-browser-plugin
   ```

2. Open **Settings → Plugins** in Buzz Desktop.
3. Select **Install** and choose `/tmp/example-browser-plugin`.
4. Review the publisher, native-code warning, and `browser.browse` permission.
5. Open the browser contribution and enter an HTTP or HTTPS address.

Settings also lets you disable, enable, or uninstall the plugin. Disable keeps
the package and permission. Uninstall removes both.

The sections below define package validation, the protocol, browser behavior,
and lifecycle rules.

## Limits

- The browser surface runs only on macOS.
- A plugin is trusted native code. Process separation does not restrict what it
  can do as your operating system user.
- Cleanup covers processes that remain in the plugin process group and can be
  signaled by Buzz. Native plugins can start processes outside that cleanup.
- This version supports one browser contribution and one `browser.browse` grant.
- Browser storage is temporary. Buzz blocks popups, downloads, camera access,
  and microphone access. User-selected file uploads still work.
- Some page loads can fail before commit without a host error. Section 5 gives
  the exact cases.
- This version has no update flow. Uninstall a plugin before reinstalling its ID.

- Package format version: `0.1.0-alpha`.
- Buzz contract version: `0.1.0-alpha`.
- Model Context Protocol (MCP) version: `2026-07-28`.
- Supported platform for the browser surface: macOS only. Opening a browser
  surface on another platform fails visibly.
- Installation requires a valid package and a target entry for the host's exact
  Rust target triple. A Darwin-only package cannot install on Linux.
- Browser surfaces remain macOS-only when a package includes another platform's
  executable.

Wire shapes below are derived from the official MCP schema for revision
`2026-07-28`
(`https://raw.githubusercontent.com/modelcontextprotocol/modelcontextprotocol/main/schema/2026-07-28/schema.json`,
`$defs`). That revision has **no** `initialize` or `initialized` call. Every
request carries its own `_meta`. Validate your frames against that schema.

## 1. Package layout

A package is a directory, not an archive. It is copied and frozen at install.

```text
my-plugin/
  manifest.json
  bin/
    aarch64-apple-darwin        # executable named by target triple
  README.md                     # optional, ignored by the host
```

Limits: at most 16 files, 64 MiB total, and 256 total file/directory entries excluding the package root. Directory nesting is limited to 16 levels below the root. Only regular files and directories are accepted.
Symlinks, devices, FIFOs, sockets, absolute paths, and `..` segments fail the
install with no state written. The host also rejects an installed plugin ID,
including a disabled installation. The existing package, grant, and session
stay unchanged. Uninstall the plugin before you install that ID again.

## 2. Manifest

You must supply each `manifest.json` field unless marked optional. The manifest
is strict. The host rejects each unlisted key at every document level. It also
rejects unknown contribution kinds and grant names.

This revision has no ignored key or free-form metadata slot. Every unrecognized
key is an error. A later revision can add fields without ambiguity.

| Field | Type | Notes |
|---|---|---|
| `packageFormatVersion` | string | must equal `"0.1.0-alpha"` |
| `contractVersion` | string | must equal `"0.1.0-alpha"` |
| `id` | string | reverse-domain, `^[a-z0-9]([a-z0-9-]*[a-z0-9])?(\.[a-z0-9]([a-z0-9-]*[a-z0-9])?)+$` |
| `name` | string | 1-64 characters, shown in Settings |
| `version` | string | three-component SemVer |
| `publisher` | string | 1-64 characters, shown in the consent dialog |
| `license` | string | SPDX identifier |
| `homepage` | string (optional) | `https` only |
| `runtime.type` | string | must equal `"stdio"` |
| `runtime.targets` | object | keys are target triples. Must include an entry for the host target. |
| `runtime.targets.<triple>.path` | string | `/`-separated path inside the package |
| `runtime.targets.<triple>.sha256` | string | 64 lowercase hex characters |
| `runtime.targets.<triple>.bytes` | integer | exact file length |
| `grants` | array of string | prototype accepts exactly `["browser.browse"]` |
| `contributions` | array | exactly one entry in this revision |
| `contributions[].kind` | string | must equal `"browser"` |
| `contributions[].id` | string | `^[a-z0-9-]{1,32}$`, unique in the package |
| `contributions[].title` | string | 1-48 characters, shown as the surface title |

`browser.browse` means: this contribution may show web pages the user asks for.
The user grants it once at install. The host decides which addresses it allows
(section 5). The plugin does not make that decision. There is no per-site list.

### Example manifest

```json
{
  "packageFormatVersion": "0.1.0-alpha",
  "contractVersion": "0.1.0-alpha",
  "id": "dev.example.webbrowser",
  "name": "Example Web Browser",
  "version": "0.1.0",
  "publisher": "Example Devs",
  "license": "Apache-2.0",
  "runtime": {
    "type": "stdio",
    "targets": {
      "aarch64-apple-darwin": {
        "path": "bin/aarch64-apple-darwin",
        "sha256": "0000000000000000000000000000000000000000000000000000000000000000",
        "bytes": 412344
      }
    }
  },
  "grants": ["browser.browse"],
  "contributions": [
    {
      "kind": "browser",
      "id": "web",
      "title": "Web"
    }
  ]
}
```

## 3. Transport

The host spawns the executable with no arguments, in its own process group,
with an empty environment except: `LANG`, `TMPDIR`, `BUZZ_PLUGIN_ID`, and
`BUZZ_PLUGIN_CONTRACT_VERSION`. `BUZZ_PRIVATE_KEY`, `BUZZ_RELAY_URL`, and
`BUZZ_AUTH_TAG` are never present.

Framing is newline-delimited JSON-RPC 2.0 on stdin and stdout. Write one UTF-8
JSON object per line, with no embedded newlines. A line over 1 MiB ends the
session and sends `SIGKILL` to the process group. Invalid objects and unknown response IDs
have the same result. Write diagnostics to stderr. The host keeps the last
8 KiB for the Buzz application log.

Stderr never reaches the user interface. The surface shows one fixed sentence
for each error code. Do not use stderr to communicate with the user. Write only
JSON-RPC to stdout.

Deadlines: `server/discover` 5 s, `tools/list` 5 s, `tools/call` 2 s. A missed
deadline cancels the request. The host discards a later response.

Your process must be trusted code. Buzz states this in the consent dialog: the
plugin runs as the same operating-system user and process separation is not a
security boundary for it. The browser webview is separate and is described in
section 5.

## 4. Required methods

Implement `server/discover`, `tools/list`, and `tools/call`. Nothing else is
called. `notifications/*` from the plugin are ignored.

### 4.1 `server/discover`

Host request:

```json
{"jsonrpc":"2.0","id":1,"method":"server/discover","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{},"io.modelcontextprotocol/clientInfo":{"name":"buzz-desktop","version":"0.5.20"}}}}
```

Your response must include `cacheScope`, `capabilities`, `resultType`,
`supportedVersions`, and `ttlMs`.

```json
{"jsonrpc":"2.0","id":1,"result":{"resultType":"complete","supportedVersions":["2026-07-28"],"capabilities":{"tools":{"listChanged":false}},"cacheScope":"private","ttlMs":0,"_meta":{"io.modelcontextprotocol/serverInfo":{"name":"dev.example.webbrowser","version":"0.1.0"}}}}
```

The host fails the session with `plugin-contract-mismatch` if
`supportedVersions` omits `2026-07-28` or `capabilities.tools` is absent.

### 4.2 `tools/list`

Host request:

```json
{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}}}
```

Your response must advertise exactly one tool named `browser.resolve`. Extra,
missing, or renamed tools fail with `plugin-contract-mismatch`. `cacheScope`,
`resultType`, `tools`, and `ttlMs` are required:

```json
{"jsonrpc":"2.0","id":2,"result":{"resultType":"complete","cacheScope":"private","ttlMs":0,"tools":[{"name":"browser.resolve","title":"Resolve an address","description":"Turn what the user typed into a URL to open.","inputSchema":{"type":"object","additionalProperties":false,"required":["context","kind"],"properties":{"context":{"type":"object","additionalProperties":false,"required":["contractVersion","pluginId","contributionId","sessionId","generation"],"properties":{"contractVersion":{"type":"string"},"pluginId":{"type":"string"},"contributionId":{"type":"string"},"sessionId":{"type":"string"},"generation":{"type":"integer","minimum":0}}},"kind":{"enum":["home","address"]},"input":{"type":"string","maxLength":2048}}},"outputSchema":{"type":"object","additionalProperties":false,"required":["outcome"],"properties":{"outcome":{"enum":["resolved","rejected"]},"url":{"type":"string","maxLength":2048},"title":{"type":"string","maxLength":200},"reason":{"type":"string","maxLength":200}}}}]}}
```

The host does not use paging. It ignores `nextCursor` in a response.

### 4.3 `tools/call`

`params` requires `_meta` and `name`. `context` is opaque except for the fields
above. Do not echo it. `kind` is `"home"` when the surface opens and
`"address"` when the user submits the address field. `input` is absent for
`"home"`.

Host request:

```json
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"browser.resolve","_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}},"arguments":{"context":{"contractVersion":"0.1.0-alpha","pluginId":"dev.example.webbrowser","contributionId":"web","sessionId":"01J9Z0","generation":3},"kind":"address","input":"example.org/docs"}}}
```

Your response must include `content` and `resultType`. Put the machine-readable
answer in `structuredContent`:

```json
{"jsonrpc":"2.0","id":3,"result":{"resultType":"complete","content":[{"type":"text","text":"https://example.org/docs"}],"structuredContent":{"outcome":"resolved","url":"https://example.org/docs","title":"example.org"}}}
```

Report your own refusal in the result. Do not use a JSON-RPC error:

```json
{"jsonrpc":"2.0","id":4,"result":{"resultType":"complete","isError":true,"content":[{"type":"text","text":"not a web address"}],"structuredContent":{"outcome":"rejected","reason":"not a web address"}}}
```

Use a JSON-RPC error only for an unknown tool name or a malformed request:

```json
{"jsonrpc":"2.0","id":5,"error":{"code":-32602,"message":"unknown tool"}}
```

The host reports other `resultType` values as `plugin-protocol-error`.
This revision does not support `input_required`.

## 5. What the host does with your answer

The host validates every `url` you return before loading it, and applies the
same rules to in-page navigations the user triggers by clicking links:

- Scheme must be `http` or `https`. The host refuses `about:`, `file:`, `data:`,
  `javascript:`, `tauri:`, `buzz-media:`, and `asset:`.
- The host refuses `ipc.localhost`, `buzz-media.localhost`, and
  `tauri.localhost` at every port. It also refuses `localhost` at port `1420`.
- The value must not exceed 2048 characters. It must parse as an absolute URL.

A refusal appears as `navigation-denied` with a fixed message. A typed address
stays visible in the address field. The error omits a refused in-page URL.
Other HTTP(S) URLs are allowed, but they can fail to load. The host does not
filter page subresources, and this contract does not claim that it does.

The host owns the address field, navigation controls, window bounds, popup
blocking, download blocking, and browser store. The browser store is temporary.
History belongs to the web engine. Back and forward use the page's session
history, including forms, fragments, and same-document changes.

The webview is not a Tauri webview. It has no inter-process communication (IPC)
bridge, injected script, or Buzz protocol. A page cannot call Buzz through the
webview. Back and forward use the browser engine's native history methods.

Buzz starts a load timeout for the home page, reload, or a submitted address
without a fragment. Submitting the last browser-reported address reloads it.
Each committed main-document load restarts the 30-second timeout. A finished
load clears it.

The API has no provisional failure event. Page links, history traversal, and
new fragment addresses can fail before commit without a host error.
Same-document changes update the address and history controls within 500 ms.
Buzz leaves the current WebKit document visible after a timeout.

Browser errors use these codes: `navigation-denied`, `load-failed`,
`plugin-timeout`, `plugin-unavailable`, `plugin-protocol-error`,
`plugin-contract-mismatch`, and `unsupported-platform`. Buzz renders fixed
messages. Plugin text does not become an error message.

Camera and microphone access are denied for every page in this surface,
unconditionally. A page that calls `getUserMedia` gets a rejection without a
permission prompt. File uploads still work. An `<input type="file">` control
opens a native file picker. Downloads and new windows are blocked.

## 6. Lifecycle you can observe

| Event | What you see |
|---|---|
| Install | Nothing. Your process does not start. |
| Surface opened | process spawned, `server/discover`, `tools/list`, then a `"home"` call |
| Address submitted | An `"address"` call. The host cancels an active call first. |
| Surface closed, disable, uninstall, community switch | stdin closes, then `SIGTERM` to your process group, then `SIGKILL` after 2 s |
| Disable | Package and grant stay on disk. Enable restarts the process without reinstalling. |
| Uninstall | package directory and grant are deleted |

Exiting non-zero or crashing mid-call produces `plugin-unavailable` in the
surface. The rest of Buzz is unaffected. Buzz restarts you on the next open, not
automatically.

The `browser.resolve` result for `kind=home` is the sole home URL source. The
manifest contains no duplicate home setting. A valid `outcome=rejected` becomes
`navigation-denied` and leaves the process available for the next call.

## 7. Complete example plugin

The example includes `build.py` with this interface:
`python3 build.py --output <package-directory> [--home-url <http-or-https-url>]`.
The default home URL is `https://example.com/`. The script validates the URL and
sets compile-time `BUZZ_BROWSER_HOME_URL`. It finds the host Rust target triple,
copies the executable, and calculates its SHA-256 and byte count.

Install the output package directory, not the source crate or Cargo target
directory. The home option belongs to the example build tool. Buzz supplies no
runtime home URL variable. A local development fixture can use
`--home-url http://127.0.0.1:<port>/home` without changing the host or wire API.

`Cargo.toml`. The empty `[workspace]` table keeps this out of any enclosing
workspace, so it builds anywhere:

```toml
[package]
name = "example-web-browser-plugin"
version = "0.1.0"
edition = "2021"

[workspace]

[dependencies]
serde_json = "1"
```

`src/main.rs`:

```rust
use std::io::{BufRead, Write};

const PROTOCOL_VERSION: &str = "2026-07-28";

fn main() {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(request) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let id = request["id"].clone();
        let response = match request["method"].as_str() {
            Some("server/discover") => discover(id),
            Some("tools/list") => tools_list(id),
            Some("tools/call") => call(id, &request["params"]),
            _ => continue,
        };
        writeln!(stdout, "{response}").ok();
        stdout.flush().ok();
    }
}

fn discover(id: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0", "id": id,
        "result": {
            "resultType": "complete",
            "supportedVersions": [PROTOCOL_VERSION],
            "capabilities": { "tools": { "listChanged": false } },
            "cacheScope": "private",
            "ttlMs": 0
        }
    })
}

fn tools_list(id: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0", "id": id,
        "result": {
            "resultType": "complete", "cacheScope": "private", "ttlMs": 0,
            "tools": [{
                "name": "browser.resolve",
                "description": "Turn what the user typed into a URL to open.",
                "inputSchema": {
                    "type": "object",
                    "required": ["context", "kind"],
                    "properties": {
                        "context": { "type": "object" },
                        "kind": { "enum": ["home", "address"] },
                        "input": { "type": "string", "maxLength": 2048 }
                    }
                }
            }]
        }
    })
}

fn call(id: serde_json::Value, params: &serde_json::Value) -> serde_json::Value {
    if params["name"].as_str() != Some("browser.resolve") {
        return serde_json::json!({
            "jsonrpc": "2.0", "id": id,
            "error": { "code": -32602, "message": "unknown tool" }
        });
    }
    let arguments = &params["arguments"];
    let url = match arguments["kind"].as_str() {
        Some("home") => Some(option_env!("BUZZ_BROWSER_HOME_URL")
            .unwrap_or("https://example.com/").to_string()),
        Some("address") => normalize(arguments["input"].as_str().unwrap_or("")),
        _ => None,
    };
    match url {
        Some(url) => serde_json::json!({
            "jsonrpc": "2.0", "id": id,
            "result": {
                "resultType": "complete",
                "content": [{ "type": "text", "text": url }],
                "structuredContent": { "outcome": "resolved", "url": url }
            }
        }),
        None => serde_json::json!({
            "jsonrpc": "2.0", "id": id,
            "result": {
                "resultType": "complete", "isError": true,
                "content": [{ "type": "text", "text": "not a web address" }],
                "structuredContent": {
                    "outcome": "rejected", "reason": "not a web address"
                }
            }
        }),
    }
}

/// Accept a full URL, or add `https://` to something that looks like a host.
fn normalize(input: &str) -> Option<String> {
    let input = input.trim();
    if input.is_empty() || input.len() > 2048 || input.contains(char::is_whitespace) {
        return None;
    }
    if input.starts_with("http://") || input.starts_with("https://") {
        return Some(input.to_string());
    }
    if input.contains("://") {
        return None;
    }
    let host = input.split(['/', '?', '#']).next().unwrap_or("");
    if host.contains('.') && !host.starts_with('.') && !host.ends_with('.') {
        return Some(format!("https://{input}"))
    }
    None
}
```

Build with `cargo build --release`. Copy the binary to `bin/<target-triple>`.
Set `sha256` and `bytes` in the manifest. Install the directory from
**Settings → Plugins** in Buzz Desktop.

## 8. Deliberate omissions

This revision has no resources, settings, secret slots, cache, selectors,
actions, background handlers, lifecycle checks, migrations, signatures, package
registry, updates, or relay access. A manifest that asks for any of them fails
to install. Those belong to the unbuilt [version-1 plugin system](plugin-system.md).
This alpha does not implement that system and is not compatible with it.
