# Buzz announcement demo

This branch includes a deterministic, relay-free workspace for recording the
Buzz announcement film.

## Launch

From this worktree, run:

```bash
just announcement-staging
```

The command launches the native Tauri staging app on this worktree's isolated
port with announcement mode enabled. The app uses the deterministic mock
workspace rather than reading or writing real staging-relay data. Reloading the
app restores the same clean demo workspace.

The local demo relay also includes a running agent named **Scout**. Configure
OpenAI or Anthropic in the Agents screen, then `@mention` Scout in
`#flight-path`, `#design`, `#marketing`, or
`#queen-bee-launch`. Scout shows a typing indicator and replies using the
selected provider with the recent conversation as context. Direct messages to
Scout work too. Agent settings are retained for reloads during the current app
session; provider keys are never written into the repository or logged by the
local provider bridge.

For a browser-only preview, use `just announcement-demo`.

## Demo workspace

- Workspace: **Honeycomb Studios**
- People: Alex Rivera (Product Lead) and nine fictional teammates across
  engineering, design, marketing, research, QA, data, support, video, and
  community
- Sections: **The Hive**, **Product**, and **Launch Swarm**
- Channels include `#announcements`, `#general`, `#flight-path`, `#design`,
  `#mobile`, `#product-ideas`, `#marketing`, and
  `#queen-bee-launch`
- Projects: `flight-path`, `nectar`, `comb-kit`, and `swarm-launch`
- Direct messages: Maya Chen, Jordan Brooks, and Priya Shah

All portraits are generated fictional people. Scout uses the default generated
identity until the announcement's final agent avatar is ready.
