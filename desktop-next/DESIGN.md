# DESIGN.md

How to design well in this client. The token registry says which value to use; this says what tokens cannot express — the judgement a designer makes without thinking and an agent gets wrong without being told. Read it before building a surface.

Run `pnpm dev` and open `/design` to see the system rendered from the tokens themselves.

## Posture

Buzz is a place where people build together and bring their agents into the room. Everyday surfaces stay quiet, crisp, and highly functional; character shows up in identity, guidance, transitions, and ceremony rather than in the chrome of ordinary work. Colour is signal, not decoration. When in doubt, the interface gets out of the way of the conversation.

## Surface and depth

- **Panels sit on the backdrop; the backdrop is a gradient.** Everything else is a panel in a different place. The navigation column is not a special kind of surface.
- **A region is separated by a soft fill, not by an outline.** Reach for `bg-inset` before reaching for a border. A bordered box announces its own edges; a filled one lets the content sit in a place. Grouping is the common case, so the quiet treatment is the default one.
- **A border is for a genuine boundary, and never above `border-secondary`.** A hairline that reads as a line — rather than as the edge where two surfaces meet — is too strong. Text and borders share the emphasis names but not their values: text lives at the dark end of the neutral ramp, borders at the light end. If a divider looks like text, it is pointed at the wrong role.
- **No page-wide gradient behind documentation or dense reading.** The gradient is the product's backdrop for chrome and panels. Behind a column of prose it fights the text and makes contrast position-dependent — such surfaces sit on `bg-panel`.
- **Shadows stay at the threshold of perception.** If a shadow is obvious, it is too strong. The two elevation values are the whole vocabulary.
- **Elevation is carried by shadow in light mode and by lightness in dark mode.** On a near-black background there is nothing darker for a shadow to cast, so a floating surface becomes a step lighter instead. Never reach for a stronger shadow to make something float in dark mode.
- **On a translucent surface, elevation reads as less translucency, not as a lighter colour.** A glass container with a fully opaque child looks layered; the same container with a merely brighter child looks unchanged.
- **Light comes from one direction, and every glass surface agrees on it.** A glass rim is bright along the lit edge and dimmer on the opposite one; that is what makes it read as a material rather than an outline. Two surfaces lit from different directions in the same view look like a mistake.
- **A glass rim is not an outline.** If a surface needs a visible boundary rather than a material edge, it wants a border role, not glass.
- **Glass needs something behind it worth seeing.** Translucency over a flat fill is wasted cost; use it where the gradient, an image, or content actually shows through.
- **Only panels and chrome should sit directly on the gradient as a default.** Text and hairlines on a gradient have position-dependent contrast. Good practice rather than a hard rule — a rotated label pill on the backdrop is fine.

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

- **A type role carries its whole setting.** Size, line height, letter spacing, and weight are one decision, not four. `text-body` alone produces correctly set text — never pair a size role with a separate weight or leading utility, because that is how two supposedly identical labels drift apart.
- **Roles are named for the job the text does, never for its size.** `text-title`, not `text-28`. A size name is a value in disguise and goes stale the moment the ramp moves.
- **Never all-caps, and never tracked-out labels.** A capitalised label is harder to read than its sentence-case version and reads as enterprise chrome. A quiet label earns its quietness from size and colour — `text-meta` on `text-tertiary` — rather than from being shouted. There is deliberately no uppercase utility in this system.
- **Every size is relative.** Nothing may be expressed in px: fixed pixel text freezes against keyboard zoom and ignores the person's font-size preference. The existing client shipped a regression from exactly this.
- **Tracking is an optical correction, not a style.** Inter needs progressively tighter spacing as it grows. The ramp already applies it per step; do not add tracking by hand.
- **Code is one step below body.** At equal size a monospace face reads larger than Inter and pulls the eye off the sentence.

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
- **Pick the frame before the content.** Decide what the surface is — a list, a reading column, a workspace — before filling it.
- **Whitespace is generous by default.** Crowding reads as a different product.

## Motion

- **Direct manipulation follows the pointer exactly, with no easing.** Smoothing during a drag or resize reads as lag. Spring physics belongs to what happens after release.
- **A drag gesture must not select text in whatever it passes over.**
- **Never animate blur.** Re-blurring a large surface every frame is expensive enough to feel. Animate opacity instead.
- **Motion explains a change; it does not decorate one.** If removing an animation loses no information, remove it.

## Colour discipline

- **Colour is signal.** Status, authorship, presence, and mentions earn colour. Ordinary structure does not.
- **Name colours after colour jobs, never after the thing on screen.** If the name is an interface element — mention, unread, badge, sidebar — it belongs in the component, assembled from roles that already exist.
- **A colour is used one of two ways: solid or tint.** Solid carries an action and takes its paired text; tint carries a meaning and takes coloured text. There is deliberately nothing between them.
- **Accent is signal, never structure.** Reaching for an accent surface where a neutral one belongs is the most common way a functional screen starts to look decorated.
- **Never use a status colour decoratively.** A green that does not mean success teaches people to stop trusting green.
- **Categorical colours are the one place appearance-naming is allowed.** Telling two projects apart genuinely is a choice about appearance.

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

1. Search the role list by intent, not by colour.
2. A state of an existing role — add the `-hover`, `-selected`, or `-disabled` sibling with both values.
3. A material variant — add the `-glass` sibling with both values and its blur token.
4. A new role using existing words — add the name, both values, a one-sentence description, and an owner.
5. A new hue — generate its ramp, add roles pointing at steps. Never a literal.
6. A new vocabulary word — allowed, but it is the thing the audit reports on its own line, so use an existing word if one fits.
7. **Never write a raw value.** If nothing above applies, say so rather than reaching for a literal.

Every addition lands in `src/shared/tokens/registry.ts` in the same change that needed it. Promotion from `proposed` to `core` is a metadata change, not a rename.

## Components

- **Compose existing components freely. Never reimplement one.**
- **Need a variant that doesn't exist? Add it, mark it proposed.** If a variant almost fits but you would cancel several of its states, the base is wrong for the job and the system is missing a variant.
- **Never add a boolean prop for a visual difference.** Variants are enumerable, so an agent can read the list and pick; booleans multiply, and nobody designed most of the combinations. New props are for data and behaviour, not appearance.
- **Used by one feature? It lives in that feature's folder.** Used by two? Propose it as shared. The folder is the namespace.

## Using the system

- **Use an existing component before creating one, and an existing role before adding one.**
- **A new visual treatment that repeats belongs in the system, not in the feature.**
- **If a screen looks right but breaks these rules, the rules are probably wrong — say so.** This document is meant to be argued with, not worked around.
