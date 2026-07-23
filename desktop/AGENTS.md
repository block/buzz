# Desktop instructions

Read the root [`AGENTS.md`](../AGENTS.md) first. Buzz Desktop is Tauri 2,
React, TypeScript, Vite, Tailwind, and Biome; see [`README.md`](README.md) for
the local frontend commands.

| Before editing | Read first |
| --- | --- |
| Playwright coverage or captured UI | [`tests/e2e/AGENTS.md`](tests/e2e/AGENTS.md) |
| Community initialization or relay changes | [`src/features/communities/AGENTS.md`](src/features/communities/AGENTS.md) |
| Agent configuration | [`src/features/agents/AGENTS.md`](src/features/agents/AGENTS.md) |

- Use rem-based, named Tailwind text tokens for readable text. Do not add
  pixel-sized or arbitrary text-size utilities: webview zoom scales rem text.
- A community switch remounts React state but not module-level state. Any new
  community-scoped cache, singleton, or long-lived store needs an explicit reset
  in `src/features/communities/useCommunityInit.ts`.
- Use `just desktop-check`, `just desktop-test`, and the relevant Tauri checks
  for desktop changes; `just desktop-ci` runs the desktop suite.
