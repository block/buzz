# AGENTS.md — desktop-new

This is a **new Buzz desktop client, built from scratch.** It is not a fork of
`desktop/` and not a migration target. The existing client stays untouched; this
one is being built to replace it eventually, starting from the design system
rather than from features.

For repository-wide rules see the root [AGENTS.md](../AGENTS.md). Everything here
is specific to this client and takes precedence within it.

---

## Read before writing any UI

1. [DESIGN.md](./DESIGN.md) — the judgement tokens cannot express. Not optional.
2. `src/shared/tokens/registry.ts` — what exists, what each name is for.
3. Run `pnpm dev`, open `/design`, and look at the thing you are about to change.

---

## Getting started

```bash
pnpm install
pnpm dev            # http://localhost:1430
```

`/design` is the design system, rendered from the token registry. `/` is a
placeholder — the app shell and its capabilities come later.

Port 1430, so it can run alongside the existing client on 1420.

```bash
pnpm typecheck      # tsc --noEmit
pnpm check          # biome + the type, contrast, and colour guards
pnpm check:type     # the type-system guard alone
pnpm check:contrast # the contrast guard alone
pnpm check:color    # the colour-system guard alone
pnpm biome check --write .   # auto-fix
```

## The guards

Three scripts, run by `pnpm check`. They read your files as text and look for a
short list of specific mistakes. Nothing depends on them — the app builds and
runs identically without them. They exist because **an agent has no basis for
choosing between two plausible values and will otherwise pick either**, which is
how two labels that should match end up different.

| Script | Rejects | Why it exists |
|---|---|---|
| `check-type.mjs` | arbitrary text sizes (px **or** rem), `uppercase`, hand-applied `tracking-*`, any font weight but 400/600 | Each one is a defect the existing client already shipped. A px size freezes against keyboard zoom. |
| `check-contrast.mjs` | any text role that fails its **APCA** target (Lc 60 body, Lc 45 meta) on a surface it can sit on, in either mode | Every role against every surface it can reach, in both modes, is more pairings than anyone checks by hand — and the old WCAG 2 ratio disagrees with the eye on dark greys. |
| `check-color.mjs` | opacity used to reach a subtler colour, `color-mix()`, alpha `rgb()`/`hsl()`, a glass fill used without its material, and **a new semantic role that is the same ramp step in both modes** | A faded colour has no name, no light/dark pair, and nothing measurable. The role check is the one that keeps the layer from regrowing: it states which class to write instead. |

`pnpm test` adds one more, and it is the one to understand before adding a check
of your own. `src/shared/tokens/registry.test.ts` asserts that **every token the
/design pages document actually exists**, and that **no text or border role
registers under Tailwind's shared `--color-*` namespace.** Both bind a
documentation claim to the stylesheet, because both had already broken:
`bg-float-glass` was described, swatched, and never declared, and `text-danger`
resolved to the saturated fill instead of the text step — shipping error text at
APCA Lc 34 while `check-contrast` passed, because it measured a token no class
could reach. **A guard that measures something the browser never resolves is
worse than no guard.** DESIGN.md § Text and borders register in their own
namespaces has the full account.

Each script's error message states the rule and the fix, so the reasoning lives
next to the code that enforces it. The design arguments behind them are in
DESIGN.md — § Type, § Contrast, § Colour discipline.

`pnpm census` is the fourth tool and the only one that never fails anything. It
lists every colour role with its readers — `var(--x)` and the Tailwind class
alike, docs counted separately from product code. Run it before adding a role, and
before arguing that one is dead.

**Every rule has a named escape hatch, and using it is a normal edit.** Add an
entry to `OVERRIDES` (or `EXCEPTIONS`) keyed by path with a one-line reason. The
guards are a note from a designer who looked at the screen, not an authority —
**if you find yourself changing a design to satisfy one, the guard is wrong.**
Edit or delete it. Stale entries are reported so the lists cannot rot.

The contrast and colour guards parse `tokens.css` rather than a copied list of
values, so they cannot drift from the tokens they audit.

---

## Colour and type systems

Both are documented in [DESIGN.md](./DESIGN.md) — § Colour structure and § Type.
They live there rather than here because they are design decisions, and because
documenting them in two places is how this file came to claim the colour system
had two layers in one section and three in another.

The one thing to know before typing: **build from the ramps.** `bg-purple-3`,
`text-red-12`, `bg-neutral-4` — every step is authored per mode, so a step behaves
correctly in light and dark. This is the opposite of the usual Tailwind advice,
and it holds only because of that per-mode authoring.

**The fifteen semantic roles are the exceptions, not the interface.** A role
exists when light and dark take *different* steps (`bg-panel` is `neutral-1` then
`neutral-3` — no class can say that), or when the name enforces a rule a ramp
cannot (there are deliberately three text levels). `pnpm check:color` rejects a
new role that is the same step in both modes, and names the class to use instead.

Glass is the one genuinely private layer: a fill without its blur, rim, and lift
is not glass, so it is reachable only through the `glass-primary` /
`glass-secondary` utilities.

---

## Stack, and why

| Choice | Reason |
|---|---|
| **Base UI** | Behaviour, accessibility, keyboard, and positioning with zero appearance. The visual language is authored here, not inherited then overridden. **Before building any interactive shared component, check Base UI first; when it has the behavior, wrap and compose that primitive rather than recreating its events, focus, positioning, portal, or dismissal logic.** A native element is appropriate only when Base UI has no corresponding primitive or the component is semantically static. |
| **Tailwind v4** | Tokens are defined in CSS via `@theme`; the CSS *is* the config. No JS config file. |
| **Own colour tokens** | Not shadcn. Its vocabulary — `muted-foreground`, `secondary-foreground` — is what made colour illegible in the existing client. |
| **TanStack Router** | File-based routes, same as the existing client. |
| **Tabler Icons** | The only icon set for product UI. Do not add or use Lucide icons. |

Astryx is a **reference** for token architecture and the agent-docs idea. Not a
dependency.

---

## Growing the system

**Need something the system doesn't have? Add it to the registry, mark it
`proposed` with an owner, keep working.** No gate, no approval, no separate
mechanism for one-offs. The full procedure is in DESIGN.md § Growing the system.

The only stop condition: **if you cannot describe it in one sentence, ask.** That
is the signal it is not a role.

---

## Structure

```
src/
  app/routes/               file-based routes
  shared/
    styles/tokens.css       the palette, the roles, the Tailwind registration
    styles/typography.css   the type ramps and the nine roles
    styles/globals.css      base styles, and the glass material utilities
    styles/components.css   the shared components' own styles
    styles/product.css      product surfaces not yet in a component
    styles/design-components.css  the /design pages' own furniture
    styles/agents.css  styles/chips.css
    tokens/registry.ts      the machine-readable colour + token description
    ui/                     the shared components (11 so far)
    ui/registry.ts          the machine-readable component description
    identity/  community/  theme/   the ambient facts — see § Capability or shared fact?
    chips/  runtime/
  features/
    design-system/ui/       the /design pages
```

`features/design-system/ui/primitives.tsx` is **documentation furniture, not a
component library.** The shared primitive layer gets built one component at a
time as the product repeats something — see DESIGN.md § Components.

---

## What is not here yet

Deliberately, so nobody assumes it was forgotten:

- **Spacing, radius, and motion tokens.** Their `/design` pages state what is
  still to decide rather than pretending to a system. Typography has landed;
  spacing is next.
- **Any product component.** No Button, no Dialog, no input layer.
- **Tauri.** This is a web app for now; the native shell comes with the app shell.
- **Relay, auth, event handling.** None of it. When it arrives it comes from the
  shared Rust crates, not a reimplementation.
- **The computed paired-text rule.** `--text-on-accent` holds a literal white
  until the lightness computation lands, and is marked as an exception. It is the
  one text colour that cannot be a ramp step: it follows its *fill*, not the mode,
  so white on `purple-9` stays white in dark mode where a neutral step would flip.
  The `bg-neutral-11` / `text-neutral-1` pair is governed by the same rule and
  measured as a pair in `check-contrast.mjs` even though both are now steps.
- **A general chrome/surface convention.** Two components now carry the axis —
  `Tabs` as `variant="chrome" | "panel"` and `IconButton` as a `chrome` variant.
  Both spell it `variant`, but nothing enforces that a component appearing on
  both backgrounds has the axis at all. See DESIGN.md § Surface and depth.
- **Contrast on glass.** `check-contrast` measures text against the opaque
  surface roles only. On a translucent surface the real background is the fill
  composited over the gradient, which changes across the screen — measured, dark
  mode's brightest glass region puts `text-secondary` at Lc 58 and
  `text-tertiary` at Lc 43 against targets of 60 and 45. Documented in DESIGN.md
  § Surface and depth with the three candidate fixes; unresolved on purpose,
  because it wants real content on screen rather than a token edit.
- **Dockview's theme in dark mode.** `DockWorkspace` passes dockview's
  `themeLight` in both modes — which is how an opaque white sheet ended up behind
  the glass until `--dv-group-view-background-color` was reset in `product.css`.
  (The shell's duplicate `dark` state is fixed: it reads `useColorScheme`, and
  `tests/sessions.spec.ts` binds the reload behaviour.)

---

## Architecture

This client is built on **composable capabilities**. A capability owns a durable product truth: it may be behavior-only,
UI-bearing, or both. A feature is an organizational boundary, a surface/view
owns spatial arrangement, and shared UI owns generic visual and interaction
primitives.

### Capability or shared fact?

App-wide things are one of two kinds, and putting the second kind in the first
kind's folder is what turns a shared layer into a dumping ground.

Ask: **could two parts of the app disagree about this, and would that
disagreement be a bug?**

- **No — it is a fact.** It answers a question and owns no policy: no fallback,
  retry, ordering, readiness, or teardown rules. `useIdentity` (who am I),
  `useCommunity` (which relay), `useColorScheme` (light or dark). Facts live in
  domain-named folders under `shared/` — `shared/identity/`,
  `shared/community/`, `shared/theme/`. There is no `facts/` folder.
- **Yes — it is a capability.** It has policy, so someone must own the single
  answer: Navigation, Conversation, Composer. The hook is only the door; the
  capability is the room behind it. A hook consumed app-wide is not evidence of
  a fact.

Direction is the invariant: **facts flow up, never down.** A capability or a
surface may read a fact; a fact must never import a capability, a feature, or a
surface. The moment a fact needs to know about Navigation, it is a capability
nobody has named yet.

**A feature reads a fact through its module, never through the backend adapter.**
Each fact exposes both shapes: `readIdentity()` / `readRelayUrl()` for an async
action, and `useIdentity()` / `useCommunity()` for render. Calling
`runtime.identity()` or `runtime.relayUrl()` from a feature is the defect — the
Agents workspace fetched both directly in two places, so a third and fourth
answer to "who am I" existed with nothing keeping them consistent. Only the facts
layer may call the adapter for a fact.

A surface must not re-derive a fact it could read. The shell held its own `dark`
boolean beside `useColorScheme` and toggled the same `.dark` class — one concept
with two owners, which is why the appearance choice did not survive a reload.
`tests/sessions.spec.ts` now binds that behaviour.

Two rules that matter from day one:

- **A capability may own live state. Any state scoped to a community must
  register its teardown** in the same change that adds it. The existing client
  learned this the hard way; do not rebuild the problem.
- **Do not create a capability speculatively.** The bar is a durable product
  identity and real composition pressure from two surfaces. Building a clean
  codebase is not a licence to relax it.

### Implementation discipline

These rules turn the general client-layer contract into decisions an agent can
apply while building Desktop New.

#### State and Effects

- **Derive values rather than synchronizing copies with Effects.** A selected
  item, lookup map, or view condition belongs in render when existing state can
  answer it; a second state value that must be kept in sync creates two owners.
- **An Effect exists only to synchronize with an external system.** Relay
  subscriptions, Tauri, browser storage, Dockview, and DOM APIs are external.
  User actions belong in event handlers or controller actions, never in render
  or an Effect that watches for them.
- **Every external synchronization has a complete lifetime.** An Effect returns
  its unsubscribe, close, disconnect, or listener cleanup. Every async result
  is fenced against the current destination and community before it writes;
  switching quickly must never let stale data land.

#### Controllers and compositions

- **A hook shares behavior, not automatically live state.** Decide deliberately
  whether a controller instance belongs to one placement, one conversation, or
  the app. A module-level store or cache is community-scoped state and must earn
  that complexity and register teardown.
- **Context distributes an existing controller inside one composition.** It does
  not create a second product authority or make a capability global by default.
- **A composition reads controller state and invokes controller actions.** It
  does not fetch backend data, call a backend adapter, or recreate policy
  locally. If a composition needs a fact, read the domain-named shared fact; if
  it needs behavior, name or extend the owning capability.

#### Reliability and test honesty

- **Every accepted user action settles visibly.** It completes, fails with an
  understandable recovery affordance, or cancels. A draft or pending action
  never clears itself into uncertainty.
- **Tests bind the product seam and must be falsifiable.** A test reaches the
  controller or composition used in the product and goes red when the guarantee
  is removed. Test rapid switching, teardown, failure, retry, recovery, and
  successful loading — not only the happy path.

#### Location while routing is incomplete

- **Navigation is the only interim location owner.** Until route-authoritative
  navigation lands, no feature or surface may introduce a second current-place
  state. When route work begins, move Navigation's existing destination model to
  the router rather than making the router and controller compete.

### Navigation capability

Navigation is a capability, not an AppShell detail. It owns the canonical
destination model, product-level navigation actions, destination resolution,
readiness, restore and fallback policy, and navigation-specific product
compositions. Put these in `features/navigation/` when this work begins.

- A feature requests a destination in product terms — for example, “open this
  channel”, “open this Session”, or “return to Messages”. It does not construct
  a URL, mutate browser history, change a Tauri window, or implement its own
  fallback/restore rule.
- AppShell places navigation compositions in the workspace and adapts their
  arrangement responsively. It does not reinterpret destination meaning or
  duplicate navigation policy.
- The Messages navigator, a future top-level lens selector, and a compact
  navigator may be separate compositions that consume the same navigation
  controller. Do not build a universal `Navigation` component with layout
  modes and optional regions.
- Generic components such as `Panel`, `Button`, `SearchField`, and
  `NavigatorRow` remain shared UI: they know nothing about Buzz destinations.
- Stop and ask before changing route shape, deep links, Back/Forward behavior,
  persisted restoration, community-switch sequencing, or navigation semantics
  shared by another client surface. These are product-contract decisions, not
  local implementation details.
