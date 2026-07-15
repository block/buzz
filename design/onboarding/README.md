# Buzz Desktop Onboarding — Design Spec Index

Source: 20 mockup PNGs (1280×832) in `~/Documents/Dev/Buzz onboarding/`.
Each spec below was produced by analyzing the source images; consult the
original PNGs (with `read_image` + crop) only when pixel-level detail is
needed.

## Flow Overview

| Step | Screen(s) | Spec | Summary |
|------|-----------|------|---------|
| 1 | `01 landing` | [spec-01-02.md](spec-01-02.md) | Centered welcome screen — bee logo, "Welcome to Buzz", yellow primary CTA + low-emphasis secondary path |
| 2 | `02a–02d profile` | [spec-01-02.md](spec-01-02.md) | Profile setup — avatar picker (photo-upload modal / emoji popover), display-name input, Continue disabled-until-filled |
| 3 | `03a–03d agents` | [spec-03.md](spec-03.md) | Agent selection — 2-col card grid with hover/selected states (accent border + checkmark, multi-select), detail modal |
| 4 | `04a–04d chat` | [spec-04.md](spec-04.md) | First-chat experience in the real app shell — canned haiku prompt, send, typing indicator, agent haiku reply → continue |
| 5 | `05a–05c harness` | [spec-05-06.md](spec-05-06.md) | Harness choice — stacked radio cards: goose (Recommended), Claude Code, Custom (ACP) |
| 6 | `06 theme` | [spec-05-06.md](spec-05-06.md) | Theme choice — Light / Dark / System preview cards, Dark preselected |
| 7 | `07a–07c` | [spec-07.md](spec-07.md) | Handoff to live app — welcome empty state with getting-started cards, seeded Inbox welcome message, starter DM conversation |

## Product Decisions (confirmed with Cynthia, 2026-07-14)

**Implementation step order differs from mockup numbering** — harness moved
before chat because the first-chat is a real agent round-trip:

1. Landing → 2. Profile → 3. Agent → 4. Harness (mock 05) → 5. First chat
(mock 04) → 6. Theme (mock 06) → 7. Handoff (mock 07)

- **Trigger:** onboarding runs whenever no identity is present — fresh setup
  or arriving via an invite link.
- **No skips for now**; agent step is never skippable. Back preserves all
  entered state.
- **Landing:** uses the marketing site's animated fuzzy bee/wordmark
  sequence. Include a low-emphasis "Already have an account?" link, stubbed
  (future: NIP-AB pairing).
- **Profile:** avatar is **required** (photo or emoji) + display name.
  Publishes standard kind-0 metadata on Continue (same path as settings
  profile edit).
- **Agent:** **single-select** (radio semantics, not the mock's multi-select
  checkmarks). Source: hardcoded curated list in
  `desktop/src/features/onboarding/agents.ts`. The chosen agent is used for
  the first-chat step and the step-7 welcome DM.
- **Harness:** choosing records the default harness in app config only — no
  install/launch during onboarding. Custom (ACP) defers configuration to
  settings later (no follow-up input).
- **First chat:** **real round-trip** with the selected agent via the chosen
  harness. Prompt is pre-filled/forced to the haiku ask. (Mock 04's Latte
  light theme is an artifact — use current app theme.)
- **Theme:** reuse the app's existing theme-selection UI. Default **System**
  (not Dark as mocked). Applies **live** on selection.
- **Handoff:** welcome message comes from the selected agent; the DM thread
  contains the real haiku exchange from step 5. Welcome/getting-started
  state **persists** (not one-time).
- **Styling:** follow the marketing designs — chartreuse `#d7d72e` / ink
  `#231e1e` palette, CashSans, fuzz/grain filters, bee field, custom cursor
  all kept. Vendor assets from `squareup/ext-builderbot-ui`
  (`src/sites/buzz/`, `public/sites/buzz/`).

## Shared Design Language

- **Theme:** Catppuccin (Macchiato dark base across most steps; step 4 shown
  in Latte light). Warm yellow/gold accent for branding + primary CTAs on
  steps 1–2; mauve/lavender accent for selection states on steps 3/5/6.
- **Wizard shell (steps 2–6):** centered ~640px column, step-dot progress
  indicator at top, title + muted subtitle, Back / Continue footer. Continue
  is dimmed until the step's requirement is met.
- **Selection cards (steps 3/5/6):** neutral hover (lighter surface, brighter
  border) vs. selected (accent border, tinted wash, radio/checkmark).
- **Step 7 needs no wizard shell** — it's the normal app shell with a
  first-run welcome empty state and seeded content.

## Implementation Notes (from specs)

- Reuse existing Catppuccin tokens and **rem-based text tokens only**
  (see AGENTS.md — px text is banned).
- Radix Dialog/Popover for the image modal (02b) and emoji picker (02c).
- Step 3 selection state: `Set<agentId>` (checkmark pattern implies
  multi-select); steps 5–6 are single-select radios.
- Step 4 should reuse the real timeline + composer components; advance the
  wizard when the agent reply renders.
- Fine copy in the PNGs was approximated in places — verify exact strings
  against the design source before shipping (flagged per-spec).
