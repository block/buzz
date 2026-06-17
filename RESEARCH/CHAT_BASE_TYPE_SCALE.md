---
title: "Chat-as-base type scale: re-anchoring the timeline type ramp"
tags: [desktop, typography, design-tokens, tailwind, zoom]
status: active
created: 2026-06-15
---

# Chat-as-base type scale

## Why this exists
tho's reframe (2026-06-15): **chat text *is* the base size** — the fundamental
building block of the app. It should not be a special between-the-cracks token
(`text-chat` = 15px, wedged between Tailwind's `text-sm` 14px and `text-base`
16px). Direction: **bump chat to 16px and make it the base**, then re-anchor the
*other* timeline/thread elements that currently sit relative to the old ~15px
chat assumption. This is a **deliberate, wider pass** — a design decision, not a
surgical bugfix.

Builds on the zoom-regression work (PR #1051, branch `tho/rem-font-zoom-fix`)
which introduced the `text-chat`/`text-code` rem tokens. That fix preserved
Kenny's 15px intent; this pass *changes* that intent to 16px-as-base.

## Non-negotiables (carry forward from the zoom fix)
- **Everything stays rem.** Cmd +/- zoom scales the root `<html>` font-size
  (rem-only by design, from #573). px would freeze against zoom — that was the
  original bug. The `check:px-text` guard already enforces this; keep it happy.
- No `html { font-size }` override exists, so root is browser-default 16px and
  rem math is standard (1rem = 16px). The `--font-size: 14px` var in globals.css
  is scoped to the emoji-picker component, NOT the app root.

## Current type landscape (timeline/thread render path)
`text-chat` (15px) is used in:
- `MessageRow.tsx` — markdown body wrapper, author name (`<span>` and `<h3>`)
- `markdown.tsx` — chat body wrapper
- `mentionChip.ts` — mention chip text
- (`text-code` 13px — inline/block code in `markdown.tsx`)
- `globals.css` `.mention-highlight` — 0.9375rem

Satellite elements currently on the **stock** scale (these are the "other
elements that sit relative to chat" tho means — review each for whether it still
reads right once chat = 16px base):
- `MessageTimestamp.tsx` — text-xs
- `SystemMessageRow.tsx` — text-xs / text-sm
- `MessageThreadSummaryRow.tsx` — text-xs
- `MessageThreadPanel.tsx` — text-sm / text-xs
- `MessageReactions.tsx` — text-xs / text-sm
- `MessageActionBar.tsx` — text-xs
- `TypingIndicatorRow.tsx` — text-xs / text-sm
- `MessageTimeline.tsx` — text-base / text-sm / text-xl (empty states / headers)
- `MessageRow.tsx` (secondary bits) — text-sm / text-xs

## The design question for the build
If chat === base (16px), the type ramp around it should be *intentional*, not
incidental. The relationships that matter:
- chat body / author = base (16px)
- timestamps, metadata, system rows, reactions = one or two steps down,
  consistently (don't leave them as ad-hoc text-xs/text-sm if the ratio now
  looks off against a 16px anchor)
- code = its own deliberate step down from chat

Open for Bart to decide the cleanest expression: lean on stock Tailwind tokens
where they land right now that base is the anchor, retire/rename `text-chat` if
it's redundant with `text-base`, and only define custom rem tokens where the
stock scale genuinely can't express the intended step.

## Out of scope
- App-wide px sweep (the ~130 px classes elsewhere). Stay in the
  timeline/thread render path + the type tokens that serve it.
- Don't widen the `check:px-text` guard roots in this pass (Marge's noted
  follow-up) unless a new render-path file demands it.

## Decision (built)
Chat === `text-base` (16px, stock). Both custom tokens **retired** — `text-chat`
and `text-code` were artifacts of the 15px-wedge era; with chat at the stock
base they're redundant. Code (inline + block) re-anchored to stock `text-sm`
(14px) — a deliberate, documented one-notch step down from the 16px base.

Satellites left on their stock tokens **deliberately**: against a 16px base the
ratios actually tighten into the standard ramp (e.g. timestamps `text-xs` 12px =
clean base→sm→xs drop, was the awkward 12/15 before). Bumping them would inflate
the UI for no design gain — the right call is to keep them and document why.

Net: zero custom font tokens. `.mention-highlight` (composer) follows chat to
`1rem` (16px). Everything rem, guard stays green.
