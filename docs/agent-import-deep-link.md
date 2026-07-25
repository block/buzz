# Agent import deep link

Buzz Desktop exposes a versioned local deep link that lets another native app
open an agent snapshot in Buzz's existing review dialog:

```text
buzz://agent-import?v=1&file=<percent-encoded-file-url>
```

Opening the link does **not** create an agent. Buzz validates and decodes the
snapshot, focuses the app, navigates to My Agents, and shows the import
preview. The user must click **Import** to create the agent.

## Recommended integration

Command-line tools should use the Buzz CLI wrapper:

```bash
buzz agents import --file ./my-agent.agent.json
```

This command is local-only and does not require `BUZZ_RELAY_URL`,
`BUZZ_PRIVATE_KEY`, or `BUZZ_AUTH_TAG`. A successful command means the
operating system accepted the request to open Buzz; it does not mean the user
confirmed the import.

Native apps may open the deep link directly. Build it with a URL library so
the nested file URL is encoded correctly. For example, in Node.js:

```js
import { pathToFileURL } from "node:url";

const deepLink = new URL("buzz://agent-import");
deepLink.searchParams.set("v", "1");
deepLink.searchParams.set("file", pathToFileURL(snapshotPath).href);
```

## Version 1 contract

- `v` is required and must equal `1`. Buzz rejects missing or unsupported
  versions.
- `file` is required and must be an absolute local `file:` URL.
- The resolved path must be a regular file ending in `.agent.json` or
  `.agent.png`.
- JSON snapshots are limited to 5 MiB. PNG snapshots are limited to 10 MiB.
- Buzz Desktop independently rechecks the path, size, extension, and snapshot
  contents before showing the preview.
- One snapshot review may be pending at a time. Reopening the same pending file
  is idempotent; a different overlapping request is rejected.

The file URL is only a handoff reference. Agent instructions and snapshot bytes
are not embedded in the deep link.

## Browser integrations

Version 1 intentionally accepts only local files. A browser cannot safely
provide Buzz Desktop with a usable local file path, and Buzz does not download
an arbitrary HTTPS URL from this deep link.

A web integration should first use a trusted native helper or CLI to download
and verify the snapshot locally, then open the v1 handoff. A future remote-file
contract should use a new protocol version and bind the URL to an expected
SHA-256 digest.
