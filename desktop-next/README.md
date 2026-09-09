# Buzz design system

The next Buzz web client starts with a living design system. `/design` is the
visual overview; `/design/components` contains 54 searchable live examples;
`/design/compositions` shows product patterns and generated responses.

```sh
. ./bin/activate-hermit
pnpm install --frozen-lockfile
pnpm --filter buzz-desktop-next dev
```

Open `http://localhost:1430/design`. Use another port when needed:
`pnpm --filter buzz-desktop-next dev --port 5188`.

## Packages and portability

- `packages/design-tokens`: semantic registry, CSS roles, typography, geometry,
  and motion. Color and type pages render from this source.
- `packages/ui`: React components, public behavior primitives, portable CSS,
  generated-response schema and renderer, and third-party notices.
- `desktop-next`: documentation and examples, consuming those packages.

`pnpm --filter buzz-desktop-next build` creates a self-contained static site in
`desktop-next/dist`, including local fonts and license notices. Serve at the host
root with an SPA fallback to `index.html` for `/design/*` routes. No authentication,
Blockcell, Storybook, private package registry, relay, or Tauri runtime is required.
The Design system GitHub workflow uploads this site as a build artifact; it does
not publish a website or an npm package automatically.

## Checks

```sh
pnpm --filter buzz-desktop-next check
pnpm --filter buzz-desktop-next test:e2e
```

The first command checks formatting, typography roles, contrast, token output,
compiled utility mappings, schema bounds, and renderer action safety. The second
builds the production app and exercises it in Chromium. Install the browser once
with `pnpm --filter buzz-desktop-next exec playwright install chromium` if needed.
See `TESTING.md` for coverage and screenshot paths.

Read `DESIGN.md` for visual rules and `OPEN_SOURCE.md` for the adaptation contract.
The existing desktop and mobile clients are not migrated by this foundation.
