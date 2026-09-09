# Buzz design system

The next Buzz web client starts with a living design system. `/design` is the
visual overview; `/design/components` contains 55 searchable live examples;
`/design/compositions` shows product patterns and generated responses.

```sh
. ./bin/activate-hermit
pnpm install --frozen-lockfile
pnpm --filter buzz-desktop-next dev
```

Open `http://localhost:1430/design`. Use another port when needed:
`pnpm --filter buzz-desktop-next dev --port 5188`.

## Focused component pages

Click a section’s eyebrow link (the category followed by →) in the catalog to open
`/design/components/<id>` (for example, `/design/components/input`). Every catalog
entry has a dedicated URL with an isolated live playground, component-specific
state guidance, and its example module. Source is shown only on these focused pages. Fields, checkboxes, switches, tabs,
progress, and meters also include side-by-side state samples using the production
components. Reset example restores both the playground and its state samples.

Every component is also indented beneath All components in the navigation, like
Token table beneath Color. The navigation list scrolls independently so theme and
density controls remain available. Use these links, the component picker, or
Previous / Next links to move through the catalog.
All components returns to that component's original catalog anchor. Direct links
and refreshes work with the same SPA fallback as the rest of the site; unknown
component IDs have a recovery link. Theme and density preferences are shared.

## Appearance

The navigation contains independent theme and Normal / Compact controls. Density
applies to every page, shared component, composition, and portaled overlay. It is
saved locally, restored before mounting, and synchronized across open tabs. An
unavailable storage write leaves the current visit usable and displays a retry
message. Normal preserves the original dimensions; Compact retains text sizes
while reducing spacing and control sizes. The Spacing and Radius pages show the
values for the selected density.

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
