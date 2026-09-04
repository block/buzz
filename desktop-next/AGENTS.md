# AGENTS.md — desktop-next

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
pnpm check          # biome + the type-system and contrast guards
pnpm check:type     # the type-system guard alone
pnpm check:contrast # the contrast guard alone
pnpm biome check --write .   # auto-fix
```

`scripts/check-type.mjs` enforces four things the type system cannot express as
a token: no arbitrary text sizes (px **or** rem), no `uppercase`, no
hand-applied `tracking-*`, and no size role paired with a weight or leading
utility. Each one is a defect the existing client already paid for.

`scripts/check-contrast.mjs` measures every text role against every surface it
can sit on, in both modes, and fails the build below its **APCA** target (Lc 60
body, Lc 45 meta). It parses `tokens.css` rather than a copied list of values,
so it cannot drift from the tokens it audits. Buzz judges contrast with APCA,
not the WCAG 2 ratio — see DESIGN.md § Contrast for the evidence, and note the
consequence: a pairing can pass WCAG AA and still fail here, which is the point.
`#8f8f8f` on `#1c1c1c` scores WCAG 5.27:1 and APCA Lc 40.

---

## Stack, and why

| Choice | Reason |
|---|---|
| **Base UI** | Behaviour, accessibility, keyboard, and positioning with zero appearance. The visual language is authored here, not inherited then overridden. |
| **Tailwind v4** | Tokens are defined in CSS via `@theme`; the CSS *is* the config. No JS config file. |
| **Own colour tokens** | Not shadcn. Its vocabulary — `muted-foreground`, `secondary-foreground` — is what made colour illegible in the existing client. |
| **TanStack Router** | File-based routes, same as the existing client. |

Astryx is a **reference** for token architecture and the agent-docs idea. Not a
dependency.

---

## The type system

Same three layers as colour, and the same rule: only roles are used when
building a screen.

```
LAYER 1   --type-size-1…9  --type-leading-*  --type-tracking-*  --type-weight-*
LAYER 2   --text-body: var(--type-size-4)   + its line height, tracking, weight
LAYER 3   text-body
```

**A role carries its whole typographic setting**, using Tailwind v4's
`--text-<name>--<property>` convention. So `text-body` sets size, line height,
letter spacing, and weight together — never pair a size role with a separate
`font-medium` or `leading-*`, because that is how two supposedly identical labels
drift apart.

Ten roles: `text-display`, `text-title`, `text-heading`, `text-subheading`,
`text-body-lg`, `text-body`, `text-label`, `text-caption`, `text-meta`,
`text-code`. Two faces: `font-sans` (Inter Variable) and `font-mono`
(JetBrains Mono) — both already shipped in every current Buzz client.

Hard rules:

- **Never a px size.** Everything derives from one virtual rem, so keyboard zoom
  and the font-size preference both work by construction. A px literal breaks
  both, which the existing client learned by shipping the regression.
- **No `uppercase`, no hand-applied `tracking-*`.** All-caps labels are less
  legible and read as enterprise chrome; tracking is already corrected per step.
- **Size roles and colour roles never collide.** Colour registers in
  Tailwind's `--color-*` namespace and is named for emphasis (`text-primary`);
  size registers in `--text-*` and is named for an editorial job (`text-body`).
  So `text-primary text-body` is one colour plus one setting, and no name ever
  means both.

### Borders register in their own namespace

`--color-*` is a single namespace, and text and borders share the emphasis names
`primary` / `secondary` / `tertiary` while holding different values — text at the
dark end of the neutral ramp, borders at the light end. Border roles therefore
register as `--border-color-*`, not `--color-border-*`.

Get this wrong and there is no error: `border-primary` silently resolves to the
*text* colour and every hairline in the product draws at near-black. That is not
hypothetical — it shipped, and it is why the first design system site had black
dividers while the tokens said `#d4d4d4`.

## The colour system

Three layers. Only the role layer is ever used when building a screen.

```
LAYER 1   --neutral-1…12  --accent-1…12  --glass-1…5  --gradient-1
LAYER 2   --bg-panel: var(--neutral-1)
LAYER 3   bg-panel
```

**Tailwind's default palette is deleted** with `--color-*: initial`. `text-gray-500`
does not exist — it is a build error, not a style choice.

Hard rules:

- **Literal values exist only in the ramps.** A component writing a hex bypasses
  the roles; a role writing a hex bypasses the ramp. Both look correct today and
  break the first theme change, and the failure hides in dark mode because light
  mode still looks right.
- **Never apply transparency to a token.** No `bg-panel/50`. Transparency lives
  inside the value. The existing client has thirteen transparencies of one grey
  and eleven of one accent because this rule did not exist.
- **Every role holds a light and a dark value under the same name.** A component
  never contains an instruction about which mode is active.
- **Accent is a slot, not a colour.** Nothing above the ramp knows the hue, so it
  can change, become a preference, or vary per theme. Do not hardcode a hue
  anywhere above layer 1.

The full exception list is in the registry and on `/design/colour`. It is short
and complete on purpose.

### Naming grammar

```
<property>-<role>[-<modifier>][-<material>][-<state>]
```

Fixed order, so there is one correct spelling. `bg-chrome-glass-hover` is legal;
`bg-chrome-hover-glass` is not. One modifier, one material, one state per name.

Every word a token may be built from is listed in `VOCABULARY` in the registry
and on `/design/vocabulary`. Combining them freely is routine. Introducing a new
word is allowed but is the thing the audit reports on its own line — use an
existing word if one fits.

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
  app/routes/           file-based routes
  shared/
    styles/tokens.css   layers 1 and 2, and the Tailwind registration
    styles/globals.css  base styles and the rim/blur/texture utilities
    tokens/registry.ts  the machine-readable system description
    theme/              colour scheme
  features/
    design-system/ui/   the /design pages
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
- **The computed paired-text rule.** `text-on-accent` and `text-on-inverse` hold
  literals until the lightness computation lands. They are marked as exceptions.

---

## Architecture

This client is built on **composable capabilities** — see the plan in Morgan's
vault. The short version: a feature owns product behaviour, a view owns
arrangement, shared UI owns visuals, and a capability owns behaviour that should
move between surfaces intact.

Two rules that matter from day one:

- **A capability may own live state. Any state scoped to a community must
  register its teardown** in the same change that adds it. The existing client
  learned this the hard way; do not rebuild the problem.
- **Do not create a capability speculatively.** The bar is a durable product
  identity and real composition pressure from two surfaces. Building a clean
  codebase is not a licence to relax it.
