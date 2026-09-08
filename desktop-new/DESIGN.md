# DESIGN.md

How to design well in this client. The token registry says which value to use; this says what tokens cannot express — the judgement a designer makes without thinking and an agent gets wrong without being told. Read it before building a surface.

Run `pnpm dev` and open `/design` to see the system rendered from the tokens themselves.

## Posture

Buzz is a place where people build together and bring their agents into the room. Everyday surfaces stay quiet, crisp, and highly functional; character shows up in identity, guidance, transitions, and ceremony rather than in the chrome of ordinary work. Colour is signal, not decoration. When in doubt, the interface gets out of the way of the conversation.

## Surface and depth

- **Panels sit on the backdrop; the backdrop is a gradient.** Everything else is a panel in a different place. The navigation column is not a special kind of surface.
- **A region is separated by a soft fill, not by an outline.** Reach for `bg-inset` before reaching for a border. A bordered box announces its own edges; a filled one lets the content sit in a place. Grouping is the common case, so the quiet treatment is the default one.
- **A border is for a genuine boundary, and there is one weight: `border-primary`.** It is `neutral-4` in light and `neutral-6` in dark. A hairline needs more separation on a dark surface than the same step number supplies: `neutral-4` measured 1.08:1 on a dark panel and 1.09:1 on the floating composer — drawn, and effectively invisible. `neutral-5` looked like the obvious one-step move but is `bg-float` in dark, so a border there would be 1.00:1: the same colour as the surface it is meant to bound. `neutral-6` clears the actual surfaces at 1.35–1.43 on panel/inset and 1.14 on float. **That mode difference is the reason this is a role rather than `border-neutral-4`.** A second weight arrives only with the design that proves a different boundary needs it. Text and borders still hold different values — text at the dark end of the neutral ramp, borders at the light end — so if a divider looks like text, it is pointed at the wrong role.
- **No page-wide gradient behind documentation or dense reading.** The gradient is the product's backdrop for chrome and panels. Behind a column of prose it fights the text and makes contrast position-dependent — such surfaces sit on `bg-panel`.
- **Shadows stay at the threshold of perception.** If a shadow is obvious, it is too strong. The two elevation values are the whole vocabulary.
- **Elevation is carried by shadow in light mode and by lightness in dark mode.** On a near-black background there is nothing darker for a shadow to cast, so a floating surface becomes a step lighter instead. Never reach for a stronger shadow to make something float in dark mode.
- **On a translucent surface, elevation reads as less translucency, not as a lighter colour.** A glass container with a fully opaque child looks layered; the same container with a merely brighter child looks unchanged.
- **Light comes from one direction, and every glass surface agrees on it.** A glass rim is bright along the lit edge and dimmer on the opposite one; that is what makes it read as a material rather than an outline. Two surfaces lit from different directions in the same view look like a mistake.
- **A glass rim is not an outline.** If a surface needs a visible boundary rather than a material edge, it wants a border role, not glass.
- **Glass needs something behind it worth seeing.** Translucency over a flat fill is wasted cost; use it where the gradient, an image, or content actually shows through.
- **Only panels and chrome should sit directly on the gradient as a default.** Text and hairlines on a gradient have position-dependent contrast. Good practice rather than a hard rule — a rotated label pill on the backdrop is fine.
- **A translucent surface has no contrast guarantee, and this one is measured.** `check-contrast` pairs each text role with the *opaque* surface roles, so glass is invisible to it — the surface a person actually reads against is the fill composited over whatever gradient happens to be behind it, which varies by position on screen. Sampled from a rendered dark-mode screenshot, primary glass over Night garden runs from `#162e28` in its quiet regions to `#1e4a3c` where the green glow reaches through. On the darker end everything clears; on the brighter end **`text-secondary` measures Lc 58 and `text-tertiary` Lc 43**, against targets of 60 and 45. Marginal, and only in a region the glow reaches — but real, and no guard can see it. Three ways out, none obviously right: make the gradients' bright stops dimmer where panels sit, raise the glass fill a ramp step under a bright backdrop, or keep meta text off glass. **Deliberately unresolved** — it needs the real product content on screen, not a token edit.
- **A redundant fill on glass is not free — it compounds.** Two identical translucent layers are not one layer: `glass-2` over `glass-2` composites to **0.77 alpha**, a value no token holds. Four panels each set the same fill as the container they exactly covered, so panels meant to be the most translucent surface in the system read as nearly solid. Before giving a region a glass fill, check whether its parent already is glass; if the region covers it, it needs no fill of its own.
- **A component that can sit on either the gradient or a panel says so, with a variant.** `Tabs` takes `chrome` (a glass pill for the app backdrop) or `panel` (an underline for a plain surface); `IconButton` has the same axis as its `chrome` variant. The failure that earned it: the chrome container is `glass-2`, which over a white panel composites to pure white, and its selected pill is `neutral-1` — also pure white. Container and selection became one colour with only a shadow between them, and no guard could see it because the component had no way to state which background it expected. **The fix was never to retint `--bg-chrome-selected`** — that moves the collision rather than removing it. **One component with a variant, not two components:** behaviour, keyboard model, accessibility, props, and the Base UI parts underneath are identical, so a sibling component would duplicate all of it to change how selection is drawn, and the two would drift exactly as the four hand-assembled chrome surfaces did. When adding a component that could appear in both places, give it the axis and put both on its specimen page — the chrome-only specimen is why this defect survived until it appeared on a real screen.

## State

- **The interface has three states, plus disabled where it matters: default, hover, selected.** There is no pressed state: pressed is too fleeting to read and makes an interface feel jumpy.
- **Hover means one step more contrast, in whichever direction that surface needs.** A light row darkens, a dark chip lightens. Direction lives in the value.
- **Selected is a persistent statement, not a stronger hover.** It should be legible without a cursor present.
- **A selected item in a toggle group is not interactive.** Clicking it does nothing, so it gets no hover.
- **Disabled communicates unavailability, not quietness.** It is not a fourth level of the emphasis ramp.
- **Never hide the only way out of a state.** Before adding a visibility rule, ask what happens when the state it assumes is wrong, and whether the person can still recover.

## Emphasis

- **Three levels of text: normal, lesser, really lesser.** If a fourth seems necessary, the thing wants a different size, weight, or position instead of a fourth colour.
- **Two text colours do most of the work.** Treat the third level as genuinely for metadata.
- **Borders use the same three levels, and they mean the same thing.** Learn the ramp once.
- **Weight and size carry hierarchy before colour does.** Reaching for a louder colour to fix hierarchy usually means the size relationship is wrong.

## Type

Two layers, and the same rule as colour: only roles are used when building a
screen. Layer 1 is the raw ramps (`--type-size-*`, `--type-leading-*`,
`--type-tracking-*`, `--type-weight-*`); layer 2 is the roles, which register in
Tailwind's `--text-*` namespace and become utilities like `text-body`. The
authoring lives in `src/shared/styles/typography.css`.

**Size roles and colour roles never collide**, because they live in different
namespaces: colour registers as `--color-*` and is named for emphasis
(`text-primary`), size registers as `--text-*` and is named for an editorial job
(`text-body`). So `text-primary text-body` is one colour plus one setting, and no
name ever means both.

Nine roles. Sans: `text-display` 32, `text-title` 24, `text-heading` 16/600,
`text-body-lg` 16, `text-body` 14, `text-body-sm` 12. Mono: `text-mono-lg` 15,
`text-mono` 13, `text-mono-sm` 11. Two faces — `font-sans` (Inter Variable) and
`font-mono` (JetBrains Mono) — both already shipped in every current Buzz client.

- **A type role carries its whole setting.** Size, line height, letter spacing, and weight are one decision, not four. `text-body` alone produces correctly set text, and its line height is never overridden — that is how two supposedly identical labels drift apart.
- **There are two weights: 400 and 600.** 400 is content — everything read. 600 is structure and emphasis: the thing that names what you are looking at, or the words a sentence leans on. `font-semibold` is what bold means here.
- **Bold body text is two utilities, composed.** `text-body font-semibold`, `text-body-sm font-semibold`. This is the one place a component adds a weight, and it is deliberate: **the size is the paragraph's decision, the weight is the phrase's.** A `text-body-bold` role would fuse them, so an agent emphasising three words would also be re-asserting a size it has no business choosing. Composing also means one rule covers every size instead of doubling the ramp.
- **500 and 700 are not in the system.** 500 was measured against 400 at body size and does not read as intent in a scanned list — subtle enough to miss, heavy enough to muddy a column. 700 is louder than anything here needs. Weight is not a ramp; it is two values with two jobs.
- **State is not weight.** Selected, active, and unread are said with colour, a fill, or a dot — all three already exist in the colour system. Reserve 600 for structure and emphasis, or it stops meaning either.
- **The guard has a named escape hatch, and using it is normal.** `scripts/check-type.mjs` rejects other weights because an agent has no basis for preferring `font-medium` to `font-semibold` and will otherwise pick either. A designer who finds a genuine optical exception adds it to `OVERRIDES` with a reason and moves on — that is an ordinary edit, not an escalation. The point is that the next person reads a decision instead of guessing at an accident. If overrides start accumulating in one direction, the system is missing a role; fix the system rather than adding a tenth entry.
- **Roles are named for the job the text does, never for its size.** `text-title`, not `text-28`. A size name is a value in disguise and goes stale the moment the ramp moves.
- **The ramp is short because the product is.** Across 73 real Buzz screens, 90% of all text is one size and two cover 93%; 16/18/20/22 together were 1.5%, scattered and inconsistent. So there is nothing between 16 and 24, and that gap is deliberate — an app has panels with names, not a document outline. Four steps above body invited a hierarchy the product does not have.
- **A heading is body-large in a different weight.** `text-heading` and `text-body-lg` are both 16px, separated by weight alone — which is what the shipping client does at 16/600, and the only text on a channel screen larger than a message. In a dense app a section name needs to be identifiable, not loud.
- **12px is the floor.** Nothing readable goes below it: the most compressed text in Berd — a timestamp in a hover slot, a model name in a 24px pill — is 12px. If something must be smaller it is not text, it is a glyph inside a component, and that component owns the size as a documented exception. Do not add a general-purpose smaller step.
- **Mono is one step below its sans partner, always.** 11↔12, 13↔14, 15↔16. At equal size a monospace face reads larger than Inter and pulls the eye off the sentence, so the correction is a rule rather than a judgement: pick the sans size, step down. Mono never exceeds body size in ordinary interface text; the one exception is a code or key a person must transcribe, which is what `text-mono-lg` is for.
- **Mono roles are named for the setting, not the content.** `text-mono`, not `text-code` — most mono in a product is a pubkey, a path, a branch name, or a hex value. Calling the role `code` made it read as a lie everywhere except an actual code block.
- **Never all-caps, and never tracked-out labels.** A capitalised label is harder to read than its sentence-case version and reads as enterprise chrome. A quiet label earns its quietness from size and colour — `text-body-sm` on `text-tertiary` — rather than from being shouted. There is deliberately no uppercase utility in this system.
- **Every size is relative.** Nothing may be expressed in px: fixed pixel text freezes against keyboard zoom and ignores the person's font-size preference. The existing client shipped a regression from exactly this. Everything derives from one virtual rem, so zoom and the font-size preference both work by construction — which is why an arbitrary rem literal is rejected too. It zooms correctly and still re-fragments the scale.
- **Tracking is an optical correction, not a style.** Inter needs progressively tighter spacing as it grows. The ramp already applies it per step; do not add tracking by hand.

## Both modes

- **Design in both modes, not in light and then dark.** Dark is not a filter applied afterwards: elevation, glass, and accent text all behave differently there.
- **Accent text moves in opposite directions between modes.** Darker than its fill on a light background, lighter on a dark one.
- **A tint is a pale wash in light mode and a deep one in dark.** The name describes the job, not the lightness.
- **Check the pairing, not the swatch.** A colour is only right in the context of what sits on it and behind it.
- **Every dark value in this system is authored rather than observed.** The design exploration it came from is light-only. Treat anything that looks wrong in dark as a finding.

## Density and rhythm

- **Dense data renders as rows with dividers, edge to edge.** Wrapping every list item in its own card is the most common way a functional surface becomes a marketing page.
- **Content that separates itself needs no divider, and no container.** A divider is for uniform rows where the eye needs a line to track along. When each entry already carries a visible difference — a colour swatch, a type specimen, an avatar — the content is the separator, and adding a rule or a card on top is redundant structure. Space alone is enough.
- **Never judge a value against a surface it will not be used on.** A swatch on a grey fill, or a type specimen in a tinted box, is being evaluated in a context the product will never reproduce. Samples sit on the page. The one exception is a value that needs a backdrop to exist at all — translucency needs something behind it, and a white surface swatch needs a hairline or it renders as nothing.
- **Cards are for widgets, galleries, and settings groups.** A card is a bordered, padded region on the page, not a different depth.
- **A card carries no default fill.** It sits on `bg-panel` and is grouped by a hairline or by spacing. Fill on a card is reserved for `bg-hover`, and only where the card is actually clickable — so a filled card always means *you are pointing at this*, never merely *this is a box*. This is why there is no `bg-card` role: a card with no default fill needs no name. It also rules out the cards-in-cards look, where a filled card inside a filled panel reads as a stack of empty text fields. Reaching for `bg-inset` here is the specific mistake — `inset` means pushed in, like an input or a code block, which is the opposite gesture from grouping.
- **Pick the frame before the content.** Decide what the surface is — a list, a reading column, a workspace — before filling it.
- **Whitespace is generous by default.** Crowding reads as a different product.

## Motion

- **Direct manipulation follows the pointer exactly, with no easing.** Smoothing during a drag or resize reads as lag. Spring physics belongs to what happens after release.
- **A drag gesture must not select text in whatever it passes over.**
- **Never animate blur.** Re-blurring a large surface every frame is expensive enough to feel. Animate opacity instead.
- **Motion explains a change; it does not decorate one.** If removing an animation loses no information, remove it.

## Colour structure

Two public layers, plus components: screens normally write a palette step, and use a role only for a decision a step cannot express.

| Layer | Example | What it is |
|---|---|---|
| **0 palette** | `--purple-9`, `--neutral-4` | Every hue, twelve steps, authored per mode. The only place a literal lives. **Public: a screen writes `bg-purple-9`.** |
| **1 roles** | `--bg-panel` | The fifteen cases a step cannot express. Public too. |
| **2 components** | `bg-panel`, `bg-neutral-4` | Tailwind utilities, from either layer. |

**Screens are built from the ramps.** This reverses the rule this file used to
state, and the reversal turns on one fact: **every palette step is authored per
mode.** `neutral-4` is `#e8e8e8` in light and `#232323` in dark, so a component
naming the step behaves correctly in both. That is what makes a raw step safe
here and unsafe in stock Tailwind, where `neutral-200` is a single literal —
naming it there really does break dark mode, and a semantic layer really is the
only fix.

Once a step is mode-aware, **a role whose light and dark values are the same step
is a name in front of a number**, and a name in front of a number hides the
decision instead of recording it. `bg-accent` was `purple-9`; `text-error` was
`red-12`. Nineteen roles were exactly that and are gone.

### When a name is earned

Three cases, and `pnpm check:color` enforces the first two by rejecting any new
role that fails them:

1. **Light and dark take different steps.** `bg-panel` is `neutral-1` in light and
   `neutral-3` in dark. No single class can say that, so the name is load-bearing.
   The four structural surfaces are all of this kind.
2. **The name enforces a rule a ramp cannot state.** There are deliberately three
   levels of text and one border weight. `text-neutral-11` looks reasonable and is
   how a fourth level appears without anyone deciding, so `text-secondary` stays
   even though its step is identical in both modes.
3. **Morgan sees a repeated pattern and asks for one.** A tinted callout that
   turns up on four screens earns a name — for the *pattern*, not the colour. This
   is the only route by which the role layer grows, and it is deliberately manual.

`neutral` is a hue like any other — the same twelve steps, the same naming. There
is no separate grey ramp and no `palette-` prefix: a step is `--neutral-4`, the
way Tailwind names a colour.

**Tailwind's default palette is deleted** with `--color-*: initial`, so
`text-gray-500` does not exist. It is a build error, not a style choice.

**There used to be a families layer** — `--accent-fill`, `--danger-tint`, thirty
steps in five families, sitting between the palette and the roles. It was
deleted. Every one of its thirty steps had exactly *one* reader, so it renamed a
colour rather than abstracting one, and answering "what colour is this button"
meant reading three lines in two places
(`bg-accent` → `accent-fill` → `palette-purple-9`). The naming survives where it
was always clearest — in the role names, which say *what the colour is for*
(`bg-accent-tint` is a background) rather than restating a job (`accent fill`
does not tell you where to put it).

**The role layer was then cut from 54 names to 15, by the same test.** A census
counted every reader of every role — both `var(--x)` in a stylesheet and the
Tailwind class each role registers as, with the /design pages counted separately
from product code, since a page displaying a swatch proves only that the token
exists. Eighteen roles had no reader anywhere and six were read only by the docs
that documented them; the rest went once palette steps became reachable as
classes and the "same step in both modes" test above disqualified them.

**The argument that lost is worth recording, because it is a good one.** A role is
a slot whose hue can change, so `bg-accent-tint` survives a retint where
`purple-3` does not. It lost to a fact: the accent hue *did* change, to Tailwind
purple, and it was five values in the ramp rather than a rename. The ramp is the
slot. A role in front of it only adds a hop.

The palette stays because it is where **light and dark are reconciled**. A hue's
dark steps are not its light steps dimmed — purple's step 12 is near-black in
light and near-white in dark; step 3 is a lilac wash in light and a deep plum in
dark. Only step 9 is identical. Two hand-authored ramps under one name is what
lets a role be a single line and still behave in both modes, and what keeps
`.dark` to a restatement of values rather than the 50 hand-picked colours it used
to hold — which is where an accent tint and a categorical purple drifted into two
different purples in dark.

To retint, change the ramp. The accent moved to Tailwind purple in five values,
which is the demonstration that **the ramp is the slot** — no rename, nothing
above it needed to know.

### Which step, for what

Twelve steps mean the same twelve jobs in every hue, so this is the map from a job
to a step. It used to generate roles; now it tells you which class to write:

| step | job | example |
|---|---|---|
| 3 | a tinted surface | `bg-purple-3` |
| 4 | that tint, hovered | `bg-purple-4` |
| 8 | border, focus ring | `border-purple-8` |
| 9 | solid fill | `bg-purple-9` |
| 10 | that fill, hovered | `bg-purple-10` |
| 12 | coloured text on a neutral surface | `text-purple-12` |

Adding a hue is mechanical — generate twelve steps, and the map above already
answers which one is the button. `cyan` and `orange` are authored and unused, so
that half is proven.

**Being mechanical is exactly why it must not run ahead of the product.** This map
is how four status identities came to exist: twenty roles from one line of a
lookup table, nineteen of which nothing ever read. It tells you which step to take
*once a design needs the colour*. It is not a licence to pre-generate a set.

**It is a good default, not a guarantee — measure the pair you actually use.**
Step 10 crosses over, darker than step 9 in light and lighter in dark, so a hover
reads as a press in light mode and a lift in dark with no special-casing. But
green's and blue's step 10 lift *too* far in dark mode and drop white text below
the APCA target. Likewise step 12 is the safe text step and step 11 is the
tempting one: red-11 is more obviously red and fails the Lc 60 body target on a
dark panel (59.7) and the dark composer (57.5), which is why error text is
`text-red-12`. A step used for text goes into `TEXT_ROLES` in
`scripts/check-contrast.mjs` so the guard measures what screens actually write.

Palette values are Radix Colors (MIT), transcribed rather than depended on —
Radix is not on Block's Tech Radar, so this is a values-only copy with no
package. Its twelve-step contract is the one this system already described in
comments, step for step. Two deliberate divergences, both documented in
`tokens.css`: `text-*` roles take step 12 rather than the 11 Radix names
"low-contrast text" (Radix sizes 11 for WCAG 4.5:1; every hue's step 11 measured
Lc 55–61 against this system's Lc 60 target), and the **neutral ramp** is
hand-authored in both modes because it was sized against the real panel stack
rather than taken from an even ramp.

### Naming grammar

```
<property>-<role>[-<modifier>][-<material>][-<state>]
```

Fixed order, so there is one correct spelling: `--bg-glass-primary-hover` is
legal, `--bg-glass-hover-primary` is not. One modifier, one material, one state per name.
This governs the fifteen roles; a ramp class is `<property>-<hue>-<step>` and has
no grammar to get wrong, which is part of its appeal.

Every word a token may be built from is listed in `VOCABULARY` in the registry
and on `/design/vocabulary`. Combining them freely is routine. Introducing a new
word is allowed but is the thing the audit reports on its own line — use an
existing word if one fits.

### Text and borders register in their own namespaces

**`--color-x` is not one utility. It is all of them.** One such line defines
`bg-x`, `text-x`, `border-x`, `ring-x` and the rest, every one pointing at the
same value. So the moment two roles differ only by *which prefix uses them*, that
namespace picks one and silently drops the other.

This has now shipped twice, and both times the symptom looked like a design
mistake rather than a registration one.

**Borders, first.** Text and borders shared the emphasis names while holding
different values — text at the dark end of the neutral ramp, borders at the light
end. Registered under `--color-*`, `border-primary` resolved to the *text* colour
and every hairline drew at near-black. It is why the first design system site had
black dividers while the tokens said `#d4d4d4`. Fixed with `--border-color-*`.

**Text colour, second, and worse.** `--color-danger: var(--bg-danger)` also
defined `text-danger`, so error text rendered in red-9 — the saturated *fill* —
instead of red-12. On a dark panel that measured **APCA Lc 34 against a target of
60**, on real error messages, for months. And `check-contrast` passed the entire
time, because it measured `--text-danger`: a token that was declared, documented,
audited, and which no class could reach.

So the rule, and it is a rule rather than a caution:

> A text role registers as `--text-color-*`. A border role registers as
> `--border-color-*`. Only backgrounds use the shared `--color-*`.

`src/shared/tokens/registry.test.ts` binds this to the file — a text or border
role registered under `--color-*` fails there now, rather than after shipping.

**The general lesson is about the guards, not the namespaces.** A guard that
measures a token nothing resolves to is worse than no guard: it reports the
system is fine and is not wrong about the token, only about whether anything uses
it. When adding a check, verify it measures the value the *browser* computes.

## Colour discipline

- **Colour is signal.** Status, authorship, presence, and mentions earn colour. Ordinary structure does not.
- **Name colours after colour jobs, never after the thing on screen.** If the name is an interface element — mention, unread, badge, sidebar — it belongs in the component, assembled from roles that already exist.
- **A colour is used one of two ways: solid or tint.** Solid carries an action and takes its paired text; tint carries a meaning and takes coloured text. There is deliberately nothing between them.
- **Accent is signal, never structure.** Reaching for an accent surface where a neutral one belongs is the most common way a functional screen starts to look decorated.
- **Never use a status colour decoratively.** A green that does not mean success teaches people to stop trusting green.
- **There are no status roles, and that is deliberate.** Danger, success, warning, and info existed as four identities of five roles each — the accent's shape copied four times, generated from one line of a lookup table. **Nineteen of the twenty had no reader outside the page that displayed them.** They were invented by symmetry rather than by need, and the symmetry actively hid the decision: the red ramp alone offers red-3, red-8, red-9 and red-12 for "an error", and a set of ready-made names made that look settled when it never was. The proof is that `text-danger` shipped resolving to the wrong red and no design had ever looked closely enough to notice. **Status colour gets designed on the screen that needs it.** Until then the ramps are right there — pick a step, measure it, and once two screens pick the same one it has earned a name.
- **Write the step, not a name for the step.** Error text is `text-red-12` and the running-agent dot is `bg-green-9`, written where they are used. Both briefly had semantic names and both were one step, identical in both modes — a name in front of a number. The name comes back if the pattern repeats across screens, and it will be named for the pattern.
- **Measure the step, do not reason about it.** Red-11 is the more obviously red choice for error text and was the first pick; measured against every surface the text actually lands on, it fails the Lc 60 body target on a dark panel (59.7) and the dark composer (57.5). Two of five surfaces — invisible to judgement, decisive on inspection. Red-12 clears all five at 82–97.
- **Categorical colours are the one place appearance-naming would be allowed.** Telling two projects apart genuinely is a choice about appearance, so a hue name is honest there. No such roles exist yet — the palette carries eight hues, and a categorical role gets named when a feature actually needs to distinguish things, not before.
- **Opacity is not how you reach a subtler colour.** If a tint looks too strong, take a different palette step — do not dim a stronger one. `purple-950/50` composites to a real, correct colour, which is exactly the trap: it is a colour decision with no name, no light/dark pair, and nothing the contrast guard can measure. A missing shade is a missing palette step, and adding one is an ordinary reviewed edit. `scripts/check-color.mjs` enforces this.
- **Transparency is a different axis from shade, and it has its own tokens.** `glass-*` exists for surfaces something must show through. Alpha baked into a named value at the palette layer is the system working; alpha applied to a token in a component is not.
- **A dark value is authored, never derived.** A hue's dark steps are not its light steps darkened or dimmed — Tailwind's purple gets *more* saturated as it descends, so a dark tint drawn from its bottom end reads as oversaturated. This is why the palette holds two authored ramps per hue rather than one ramp and a transform.
- **Two tokens doing the same job must resolve to the same step, not merely to the same value.** Matching literals drift; a shared reference cannot. An accent tint and a hand-picked categorical purple were the same colour in light mode and two different colours in dark, and nothing caught it because both held their own value.

## Contrast

Buzz judges contrast with **APCA** (the perceptual algorithm in the WCAG 3
draft), not the WCAG 2 ratio. Target **Lc 60** for body text, Lc 45 for large
or non-essential text. This is a deliberate position, taken with evidence, and
it is the rule a generated theme is measured against.

- **Why.** The WCAG 2 ratio underweights blue and ignores polarity, so it
  systematically recommends dark text on saturated mid-tone fills where light
  text is plainly more readable. Measured: white on `#3b82f6` scores WCAG 3.68
  (fail) but APCA Lc 69 (pass); black on the same fill scores WCAG 5.71 (pass)
  but Lc 40 — badly unreadable. Apple ships white on `#0088ff`–`#3daefc` in
  Messages at WCAG 2.4–3.5, and Tailwind, Bootstrap, and Radix all ship white
  on their primary blue below or near the WCAG threshold. Three independent
  signals agree with the eye; one number disagrees with all of them.
- **APCA is not the looser choice.** It is stricter wherever WCAG 2 is
  permissive: red on black (WCAG 5.25 pass, Lc 38 fail) and every dark-mode
  mid-grey. Adopting it tightens more pairings than it relaxes.
- **Report both.** WCAG 2 is what an audit measures and what regulators
  recognise today. Design to APCA, and know the WCAG number before shipping a
  surface that will be scanned. Where they disagree, say so in the change.
- **Constrain the fill, never degrade the text.** If neither black nor white
  carries a fill legibly, the fill is wrong — it is not a valid solid. Move the
  fill's lightness and keep the hue; do not settle for the less-bad text.
- **A paired text token is derived, not authored.** `text-on-*` is a function of
  its fill, so it is generated with the fill and never hand-set. Every hand-set
  pairing in this system has been wrong at least once.
- **One implementation of the rule.** Desktop, mobile, and web must not each
  compute their own pairing; they diverge and the same defect ships three times.
- **Size a text step against the worst surface it can land on**, not the most
  common one. `bg-float` is the lightest dark surface, so it is the binding case
  in dark mode; a step that only clears the target on `bg-inset` fails wherever
  a popover opens.
- **Dark mode is not light mode inverted.** APCA is polarity-asymmetric:
  light-on-dark needs more separation than the same WCAG ratio suggests. The
  dark ramp's text steps are therefore lighter than a mirrored ramp would put
  them — steps 9 and 10 sit above where linear spacing would.
- **`text-disabled` is deliberately below target.** Low contrast is the signal
  that a control is unavailable. Never put information a person needs there.
- **`pnpm check:contrast` enforces this.** Every text role is measured against
  every surface it can sit on, in both modes, parsed from `tokens.css` so the
  check cannot drift from the tokens. Exceptions live in that script with a
  stated reason, which keeps the list short and arguable.
- **A tint's hover is the hardest surface an identity has**, so a `text-*` role is
  sized against that rather than against the neutral panel. Every failure the
  audit found on a coloured surface was on a tint-hover, never at rest.
- **Hairline dividers are not held to a contrast target.** WCAG's 3:1 non-text
  rule covers boundaries needed to identify a *control* or its state, not
  grouping lines. Buzz's borders measure 1.2–1.8:1, which is where Radix and
  Apple ship theirs; raising them would draw the box the fill already implies.
  When the input layer lands, a control's own outline is a different question and
  does need the 3:1 treatment.

## Writing

- **Every word earns its place.** Prefer the shortest phrasing that stays accurate.
- **Labels say what happens, not what the thing is called internally.**
- **Empty states say what this place is for and what to do next.** An empty state is a first impression, not an error.
- **Errors say what happened and what to do about it.** A message the person cannot act on is decoration.

## Accessibility

- **Every interactive element has explicit assistive semantics, and one owner per label.** Two widgets claiming the same label produces duplicate screen-reader stops.
- **Contrast comes from the paired token, not from judgement.** Where a background is not neutral, its text is named for it.
- **Keyboard, pointer, and shortcut paths must not diverge.** When adding an input handler, enumerate the ways a person can reach it and check the ones that are not the mouse.
- **Focus is always visible.** The focus ring is part of the design, not an artefact to suppress.
- **Colour is never the only carrier of meaning.** Pair it with text, shape, or position.

## Responsiveness

- **Design for narrow, intermediate, and wide, not just wide.** Intermediate widths are where layouts usually break.
- **Text scales with the person's preference and with zoom.** Anything readable uses relative units; fixed pixel text freezes and breaks zoom.

## Growing the system

Need something the system doesn't have? **Add it, mark it `proposed`, keep working.** There is no gate and no separate mechanism for one-offs — the moment the legal path is slower than writing a raw value, the system starts being bypassed.

1. Search the component list, then the role list, by intent — not by colour.
2. If the decision is one ramp step in both modes, write that step directly. `bg-purple-3`, `text-red-12`, `border-purple-8`: the steps are public and mode-aware.
3. A state of an existing role — add the `-hover`, `-selected`, or `-disabled` sibling with both values, only if that state cannot be one step in both modes.
4. A material variant — add a named utility that carries its inseparable parts together. Glass is the example: its fill is deliberately not reachable alone, because fill without blur, rim, and lift is not glass.
5. A new role using existing words — only where one step cannot express both modes, or where the name enforces a rule. Add the name, both values, a one-sentence description, and an owner.
6. A new hue — generate its ramp. Never write a raw literal in a component; the palette is where literals live.
7. A new vocabulary word — allowed, but it is the thing the audit reports on its own line, so use an existing word if one fits.
8. **If none fit, stop and ask.** The answer is a proposed decision, not a raw value or an undocumented local exception.

Every addition lands in `src/shared/tokens/registry.ts` in the same change that needed it. Promotion from `proposed` to `core` is a metadata change, not a rename.

## Components

- **Compose existing components freely. Never reimplement one.**
- **Focus is keyboard-only visual navigation.** Pointer focus stays quiet; keyboard navigation gives the focused control—not its container—a visible focus ring. Browsers treat clicked text fields as `:focus-visible`, so the app-root keyboard-navigation fact gates every focus treatment. Never add a `:focus-within` focus ring to a container: it duplicates the child control's signal and makes pointer focus noisy.
- **Base UI is the behavior layer.** Before writing an interactive shared component, inspect Base UI for the matching primitive. When one exists, wrap and compose it; Base UI owns focus, keyboard behavior, positioning, portals, and dismissal, while Buzz owns the visual language and product semantics. Reach for native elements only when Base UI has no matching primitive or the component is semantically static.
- **Need a variant that doesn't exist? Add it, mark it proposed.** If a variant almost fits but you would cancel several of its states, the base is wrong for the job and the system is missing a variant.
- **Never add a boolean prop for a visual difference.** Variants are enumerable, so an agent can read the list and pick; booleans multiply, and nobody designed most of the combinations. New props are for data and behaviour, not appearance.
- **Used by one feature? It lives in that feature's folder.** Used by two? Propose it as shared. The folder is the namespace.

## Using the system

- **Use an existing component before creating one, and an existing role before adding one.**
- **A new visual treatment that repeats belongs in the system, not in the feature.**
- **If a shared role fails in a real context, repair the role — never work around it locally.** A documentation specimen frame needed a border but `border-primary` was neutral-4 in both modes, which measured 1.08:1 on the dark page. The wrong response was the one we made first: name `neutral-6` directly and call documentation furniture a special case. The right response was to ask whether the one shared boundary role was wrong, measure it on every surface it reaches, and make it `neutral-4` light / `neutral-6` dark. The frame then returned to `border-primary`, and every product divider improved with it. **A local exception is evidence the shared decision is incomplete, not a licence to bypass it.**
- **When choosing a colour, surface, or boundary, use this order.**
  1. **Is there already a component for the thing?** Use it. Its variants are the decisions already made. Do not assemble its fill, border, shadow, or states yourself.
  2. **If the component has no variant for its background, add a variant rather than a sibling component.** `Tabs` is `chrome` (glass pill on the gradient) or `panel` (underline on a plain surface): same behaviour, different appearance. A second component would duplicate its keyboard and accessibility contract just to change styling.
  3. **If building a surface directly, choose its job before its colour.** Backdrop → `bg-app`; opaque region on it → `bg-panel`; region pushed in → `bg-inset`; thing floating above → `bg-float`; something the backdrop should show through → `glass-primary` or `glass-secondary`. Do not use a border to do the work of an inset fill.
  4. **If drawing a boundary, use `border-primary`.** It is the one shared hairline, already authored for both modes. If it does not read in the actual context, measure that context and fix this role — do not name a neutral step at the call site.
  5. **If choosing an accent or status colour, choose a ramp step and measure it.** `bg-purple-9`, `bg-purple-3`, `text-red-12`, `border-purple-8`: every step responds to mode. Do not invent a semantic name for one choice; a repeated *pattern* earns a name when Morgan says it does.
  6. **If the choice cannot be expressed by one step in both modes, make a proposed role with both values and a one-sentence job.** If the name only restates one step, it has not earned a role. If it is a whole treatment — glass is the example — make a utility that carries every inseparable part together.
  7. **If none of this feels clearly right, stop and ask.** Choosing a raw literal or a local exception is never the escape hatch. The system is deliberately allowed to grow; uncertainty is evidence of a missing decision, not a prompt to hide one.
- **If a screen looks right but breaks these rules, the rules are probably wrong — say so.** This document is meant to be argued with, not worked around.
