# Buzz design tokens

Portable, locally owned design foundations. Color and typography CSS hold the authored values; the role registry describes their purpose. Geometry and motion CSS are generated from foundations.ts. Consumers use public roles, never private ramps.

Import the CSS exports with Tailwind v4. The UI package will also provide compiled CSS for applications without Tailwind. Switch color scheme with a dark class on the root. Fonts are supplied by the consuming application; the reference uses Inter Variable and JetBrains Mono from public packages.

Run the repository-pinned Node against build.mjs to regenerate foundation CSS; --check detects drift.

## Density

Normal is the default. Set `data-density="compact"` on the root `<html>` element
for Compact; remove it or set `data-density="normal"` to restore Normal. The root
scope includes portaled menus, dialogs, and toasts. This works with both the
Tailwind source exports and the UI package's compiled CSS.

`FOUNDATION_GROUPS` holds Normal values; `COMPACT_FOUNDATIONS` holds the proposed
Compact overrides in the same source file. The generator emits both. Controls,
padding, grouped spacing, multiline fields, chips, and surface radii use these
roles. Tailwind's `--spacing` layout rhythm changes from 0.25rem to 0.1875rem so
utility-based layouts participate as well. That includes numeric width/height
utilities: use an explicit semantic dimension when a region must keep its size
(the catalog's navigation rail does). Content-driven and reading widths remain
independent of density.

Typography, icons, color, motion, scrollbars, checkable control marks, and resize
hit areas keep their values. Root font-size still scales both profiles with zoom;
`--type-scale` remains the separate text preference. Consumer applications own
persistence; no browser storage is accessed by the token or component packages.
