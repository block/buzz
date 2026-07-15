# Onboarding Spec — Step 4: First Chat Experience

Source mockups (1280×832 each):

| File | State |
|------|-------|
| `04a chat.png` | Empty chat — user has just arrived in the channel, composer empty |
| `04b chat prompt.png` | Prompt typed into the composer, not yet sent |
| `04c chat sent.png` | User message appears in the timeline; agent typing/working |
| `04d haiku.png` | Agent responds with a haiku; onboarding step complete |

All four screens share the same frame; 04a is documented in full and b/c/d are
described as diffs against it.

---

## 04a — Base screen: empty chat

### Overall layout

The screen is the full Buzz desktop shell (Tauri window, 1280×832) with an
onboarding overlay treatment:

- **Left sidebar (~256 px, full height).** Standard Buzz workspace sidebar:
  workspace name at top, channel list below, user identity block at bottom.
  The sidebar is rendered dimmed / de-emphasized relative to the main pane —
  it is visible for context but is not the focus of this step (a subtle
  scrim/opacity reduction keeps attention on the chat pane).
- **Main pane (remaining ~1024 px).** Split vertically into:
  1. **Onboarding header band** across the top (~140–160 px): step title,
     supporting copy, and the step progress indicator.
  2. **Chat timeline region** (the large middle area): empty in 04a, showing
     an empty-state illustration/hint centered in the region.
  3. **Message composer** docked at the bottom (~72–90 px tall including
     padding), full width of the main pane with horizontal insets.
- Content in the main pane is centered on a comfortable measure (~640–680 px
  column) rather than stretching edge-to-edge; the composer spans the same
  column width.

### Onboarding header

- **Step indicator / stepper:** a horizontal dot-or-segment progress indicator
  showing 4 steps with the **4th step active** (filled/accent) and steps 1–3
  shown completed (filled or checked, muted accent). Positioned above or
  beside the title, top of the main pane.
- **Heading:** large, bold, dark-on-light — copy along the lines of
  **“Say hello to your agent”** / “Send your first message”.
- **Subcopy:** one line of muted secondary text beneath the heading explaining
  that messages in this channel go to the AI agent and inviting the user to
  try a prompt.

### Chat timeline (empty state)

- Centered empty-state block in the timeline region:
  - Agent avatar (circular, ~48–64 px) — a distinct agent identity mark.
  - A short friendly line of muted text inviting the first message (e.g.
    “No messages yet — start the conversation below”).
- No message bubbles, no typing indicator, no timestamps in this state.

### Composer

- Rounded-rectangle input (large radius, ~12 px), 1 px border in a muted
  outline color, slightly elevated on the pane background.
- **Placeholder text:** muted, e.g. **“Message …”** with a suggested-prompt
  flavor (“Ask for a haiku…” style hint — the mock uses a haiku prompt as the
  canned suggestion, see 04b).
- Left side: attachment “+” affordance. Right side: **send button** — circular
  or pill, rendered **disabled/muted** while the field is empty.
- A suggested-prompt chip/hint may sit above or inside the composer nudging
  the user toward the exact prompt typed in 04b.

### Navigation

- **Back** affordance (text link or chevron) at the lower-left / header-left
  to return to Step 3.
- **Skip** (muted text link, top-right or bottom-right) to bypass the chat
  demo.
- There is **no explicit “Next” button** in 04a — advancing is driven by the
  interaction itself (sending a message → receiving the reply), after which a
  continue affordance appears (see 04d).

### Colors / theme

- Light theme consistent with **Catppuccin Latte**: warm off-white base
  (`#eff1f5`-family) background, slightly darker panel tint for the sidebar,
  near-black/very dark slate text (`#4c4f69`-family), muted gray secondary
  text (`#6c6f85`-family).
- Accent color on the active stepper segment, send button (when enabled), and
  agent identity — a saturated Latte accent (blue/lavender family).
- Borders/hairlines are low-contrast warm grays. Corners are generously
  rounded throughout (cards, composer, bubbles ~10–16 px).

### Spacing / sizing impressions

- Generous vertical whitespace between header, timeline, and composer.
- Column max-width ~640–680 px keeps line lengths readable.
- Type ramp: heading ≈ 24–28 px bold; body/subcopy ≈ 14–16 px; composer text
  ≈ 15–16 px. (Implementation note: use rem tokens — `text-base` for chat
  body per desktop conventions.)

---

## 04b — Prompt typed (diff vs 04a)

- **Composer is focused and populated.** The placeholder is replaced by the
  typed prompt — the canned onboarding prompt, along the lines of:
  > **“Write me a haiku about getting started.”**
  (dark primary-text color, cursor at end; focus ring / stronger border on
  the composer).
- **Send button activates:** switches from muted/disabled to the filled
  accent color, clearly tappable.
- Timeline region still shows the 04a empty state (unchanged).
- Header, stepper, sidebar, navigation: unchanged.
- If 04a showed a suggested-prompt chip, it is now consumed/hidden.

## 04c — Message sent (diff vs 04a)

- **Timeline now contains the user’s message:**
  - Right-aligned (or author-labeled) message bubble containing the exact
    prompt text from 04b.
  - User avatar/initial beside the bubble; small muted timestamp.
  - Bubble style: filled tint (accent-tinted or neutral card) with rounded
    corners, matching Buzz message styling.
- **Agent activity indicator** below the user message: agent avatar +
  **typing indicator** (three animated dots) or a “thinking…” status row in
  muted text — communicates the agent is composing a reply.
- **Composer resets to empty** — placeholder text returns, send button back
  to the muted/disabled state.
- Empty-state block from 04a is gone (replaced by the timeline content).
- Header/stepper/sidebar/navigation unchanged.

## 04d — Haiku response (diff vs 04c)

- **Agent reply bubble** appears beneath the user message, left-aligned with
  the agent avatar and name:
  - Content is a **three-line haiku** rendered as three short stanza lines
    inside the bubble (themed around beginnings/getting started, matching the
    prompt). Muted timestamp adjacent.
  - Bubble uses the neutral/incoming style (surface card, subtle border) as
    opposed to the user’s tinted outgoing bubble.
- **Typing indicator removed** (replaced by the reply).
- **Completion affordance appears:** the onboarding step resolves — a
  primary **“Continue” / “Finish” / “Get started”** button (filled accent,
  pill/rounded) is now shown (header area or beneath the conversation),
  and/or the header copy updates to a congratulatory line (e.g. “Nice — you
  just talked to your first agent”). The stepper shows step 4 complete.
- Composer remains available (empty) — reinforcing that the user can keep
  chatting.
- Sidebar and overall frame unchanged.

---

## Interaction flow summary

```
04a (empty)          04b (typed)           04c (sent)             04d (reply)
─────────────        ─────────────         ─────────────          ─────────────
empty timeline  →    prompt in composer →  user bubble +      →   agent haiku bubble +
disabled send        send enabled          typing indicator       continue/finish CTA
```

## Implementation notes for Buzz desktop

- Reuse the real message timeline + composer components (`desktop/src/features/`)
  rather than bespoke onboarding widgets, wrapped in an onboarding layout
  shell providing the header/stepper/scrim.
- The agent reply should come from a scripted/seeded flow (mock or a real
  agent invocation) — the mock’s haiku is placeholder copy; final copy TBD.
- All text sizes in rem tokens (`text-base` chat body, `text-2xs` timestamps)
  per AGENTS.md zoom rules; colors from the Catppuccin Latte/Macchiato token
  set so the step also works in dark mode.
- Advance condition for the step: first agent reply rendered → show the
  continue CTA (04d), not merely message sent.
