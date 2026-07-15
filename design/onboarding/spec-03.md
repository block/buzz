# Onboarding Spec — Step 3: Agent Selection

Source mockups (1280×832 each):

- `03a agents.png` — base agent-selection grid (no interaction)
- `03b agent detail.png` — agent detail overlay/modal
- `03c agent hover.png` — hover state on an agent card
- `03d agent selected.png` — selected state on an agent card

---

## 03a — Base screen: agent selection grid

### Overall layout

Full-window onboarding screen, dark theme. Three vertical regions, all
horizontally centered on the window's center axis:

1. **Header block** (top ~25% of the window): step indicator, page title,
   supporting subtitle. All text center-aligned.
2. **Content region** (middle ~55%): a centered grid of agent cards, laid out
   as a **2-column grid** with equal-width cards (each roughly 320–340px wide,
   ~170–190px tall), consistent gutters (~20–24px) between columns and rows.
   The grid as a whole occupies roughly the middle 700px of the 1280px canvas,
   leaving wide symmetric margins.
3. **Footer bar** (bottom ~15%): navigation controls — a secondary
   **Back** affordance on the left side of the centered content column and a
   primary **Continue / Next** button on the right, plus a skip-style text
   link. Footer controls align to the same content column edges as the grid.

There is no app sidebar or chrome — this is a dedicated, pre-workspace
onboarding surface rendered edge-to-edge.

### Header components & copy

- **Stepper / progress indicator**: a horizontal row of small step markers
  (dots/segments) centered above the title, with the third step visually
  active (filled with the accent color; prior steps shown as
  completed/filled, later steps dimmed). This confirms the screen is
  step 3 of a multi-step flow.
- **Title (H1)**: a large heading inviting the user to pick agents —
  "Choose your agents"-style copy, set in a bold, large (~28–32px
  equivalent) weight in near-white text.
- **Subtitle**: one line of muted, smaller (~14–15px equivalent) supporting
  copy beneath the title explaining that agents are AI teammates that can be
  added to the workspace now and changed later. Rendered in the theme's
  subtext gray.

### Agent cards (grid content)

Each card in the grid follows the same anatomy:

- **Container**: rounded-rectangle card (radius ~10–12px) with a surface
  background one step lighter than the page background and a 1px hairline
  border in a subtle gray. Internal padding ~16–20px.
- **Avatar / icon**: top-left of the card — a rounded square or circular
  avatar tile (~40–48px) containing the agent's mark (emoji/logo glyph),
  each agent's tile tinted a different accent hue.
- **Agent name**: bold, base-size near-white text next to or below the
  avatar (e.g. the flagship in-house agent — Goose — plus other
  well-known coding agents such as Claude Code / Codex-style entries).
- **Description**: 1–2 lines of muted small text summarizing what the agent
  does (coding/automation/general-assistant style blurbs), truncated to fit
  the card height.
- **Metadata badges**: small pill badges along the bottom of the card
  (rounded-full, tiny text, tinted backgrounds) tagging capability/category
  (e.g. "Coding", "Recommended"-style tags). One card in the grid carries a
  highlighted "Recommended" treatment.
- **Affordance hint**: the card as a whole is the click target; a subtle
  chevron or info affordance in the card's corner hints that a detail view
  is available (this is what opens 03b).

Cards are visually uniform in size regardless of description length —
descriptions clamp rather than growing the card.

### Footer / navigation

- **Back**: ghost/secondary button (text + optional left chevron) on the left
  edge of the content column, muted-gray text — returns to Step 2.
- **Primary CTA**: solid accent-colored button (rounded, ~40px tall,
  white/near-white label) on the right edge — advances to Step 4. In the base
  03a state (nothing selected) it reads as the flow's forward action;
  contrasted with 03d it appears in its default/less-emphasized state until a
  selection exists.
- **Skip link**: small muted text link ("Skip for now"-style) allowing the
  user to bypass agent selection, positioned near the primary CTA / centered
  beneath the buttons.

### Colors / theme

Consistent with Catppuccin **Macchiato** (the Buzz dark theme):

- Page background: deep blue-gray base (≈ `#24273a` base / `#1e2030` mantle).
- Card surfaces: slightly lighter surface tone (≈ `#363a4f` surface0) with
  hairline borders (≈ overlay0 at low opacity).
- Primary text: near-white (≈ `#cad3f5` text).
- Secondary text: muted gray-lavender (≈ `#a5adcb` subtext0).
- Accent: the Buzz brand accent (mauve/blue family) used for the active
  stepper segment, the primary CTA, badge tints, and (in 03c/03d) the
  hover/selected borders.
- Avatar tiles: per-agent tinted hues drawn from the Catppuccin accent
  palette (green/teal/peach/mauve family), giving each agent a distinct
  identity color.

### Spacing / sizing impressions

- Generous whitespace: ~80–100px top margin above the stepper; ~24–32px
  between title and grid; ~24px grid gutters; ~40–60px between grid and
  footer.
- Type ramp matches the desktop app conventions: H1 ≈ 2xl, card name ≈
  base/semibold, descriptions ≈ sm, badges ≈ 2xs–xs.
- Everything sits on an 8px-ish spacing rhythm.

---

## 03b — Agent detail (diff vs 03a)

A **detail overlay/modal** opened by clicking a card in 03a.

- **Scrim + modal**: the 03a grid remains underneath, dimmed by a dark scrim.
  A centered modal panel (~560–600px wide, tall — roughly 60–70% of window
  height) sits on top with a raised surface color, larger corner radius, and
  a stronger border/shadow than the grid cards.
- **Modal header**: the selected agent's avatar tile (larger, ~56–64px),
  its **name** as the modal title, and a short tagline; a **close (×)**
  control in the top-right corner of the panel.
- **Body**: expanded description — several lines of muted body text going
  beyond the card blurb, followed by a structured section listing the
  agent's **capabilities/features** (bulleted or icon-led rows: what the
  agent can do — coding tasks, tool use, etc.) and the same category
  **badges** seen on the card, now with room to show all of them.
- **Metadata rows**: small label/value rows (provider/model/version-style
  details) in subtext gray.
- **Modal footer** (bottom of panel, right-aligned): a secondary
  **Cancel/Back** ghost button and a primary accent **Add/Select agent**
  button. Selecting from here returns to the grid with the card in its
  selected (03d) state.
- Header/footer of the underlying onboarding page (stepper, page CTA) are
  unchanged but non-interactive behind the scrim.

## 03c — Hover state (diff vs 03a)

Identical layout and content to 03a except for **one card** (top-left of the
grid) showing hover styling:

- **Border** brightens from the hairline gray to a more visible
  accent-tinted border.
- **Background** lifts one step (surface0 → surface1) — a subtle luminance
  increase, not a color change.
- A soft **shadow/glow** appears around the hovered card, and the cursor
  affordance implies clickability.
- No change to text, badges, footer, or the other cards. No checkmark —
  hover is visually lighter-weight than selection so the two states remain
  distinguishable.

## 03d — Selected state (diff vs 03a)

Identical layout to 03a except:

- The **same top-left card** now shows the **selected** treatment:
  - A **2px accent-colored border** (stronger and more saturated than the
    hover border) around the card.
  - A **checkmark indicator** — a small filled accent circle with a white
    check glyph — anchored in the card's top-right corner.
  - A faint accent-tinted background wash over the card surface.
- **Footer changes**: the primary **Continue** button is now fully
  enabled/emphasized (solid accent fill at full opacity), reflecting that a
  valid selection exists; it may also surface a selection count. Back and
  the skip link are unchanged.
- Selection appears to be **multi-select** (checkmark pattern rather than
  radio), so multiple cards could carry this state simultaneously.

---

## State model summary

| State    | Border                  | Background            | Indicator            | Footer CTA        |
|----------|-------------------------|-----------------------|----------------------|-------------------|
| Default  | 1px hairline gray       | surface0              | —                    | default           |
| Hover    | 1px accent-tinted       | surface1 (lifted)     | glow/shadow          | unchanged         |
| Selected | 2px solid accent        | accent-tinted wash    | ✓ badge, top-right   | enabled/emphasized|
| Detail   | (modal over scrim)      | raised panel          | Add/Select in modal  | behind scrim      |

## Navigation

- **Previous**: footer "Back" → Step 2.
- **Next**: footer primary CTA → Step 4 (enabled/emphasized once ≥1 agent
  selected; skip link available to proceed with none).
- **Detail**: card click → 03b modal; modal "Add/Select" → selected state
  (03d); close/cancel → back to grid unchanged.

## Implementation notes (Buzz desktop)

- Theme tokens should map to the existing Catppuccin Macchiato palette
  already used by the desktop app — no new colors needed.
- All text sizes must use rem-based Tailwind tokens (`text-2xl`, `text-base`,
  `text-sm`, `text-2xs`) per the desktop zoom rules — no px literals.
- Card grid: CSS grid, `grid-cols-2`, fixed row height with line-clamped
  descriptions.
- Selected state is additive (multi-select) — model as a `Set<agentId>` in
  React state; Continue disabled-until-selected unless skipped.
