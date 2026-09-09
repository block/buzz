# Buzz design tokens

Portable, locally owned design foundations. Color and typography CSS hold the authored values; the role registry describes their purpose. Geometry and motion CSS are generated from foundations.ts. Consumers use public roles, never private ramps.

Import the CSS exports with Tailwind v4. The UI package will also provide compiled CSS for applications without Tailwind. Switch color scheme with a dark class on the root. Fonts are supplied by the consuming application; the reference uses Inter Variable and JetBrains Mono from public packages.

Run the repository-pinned Node against build.mjs to regenerate foundation CSS; --check detects drift.
