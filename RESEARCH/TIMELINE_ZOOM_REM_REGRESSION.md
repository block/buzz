# Timeline Zoom Regression — Findings (for Bart)

## The bug
Cmd +/- zoom no longer scales message-timeline & thread text.

## Why (mechanism)
`desktop/src/app/useWebviewZoomShortcuts.ts` scales the ROOT `<html>` font-size
(rem-based scaling) and pins native `webview.setZoom(DEFAULT)`. So only **rem**
sizes scale; hardcoded **px** sizes are frozen. This is the intended approach
("only text should scale" — keeps webview coordinate system stable). Do NOT
revert to native webview zoom.

## Two-layer history
1. **#573** (9e76a08a, May 14) — switched zoom native→root-font-size/rem-only.
2. **#891** (45f3dfe5, Jun 8, "Tune chat text sizing", klopez4212) — the recent
   timeline regression. Converted timeline rem→px.

## Kenny's intent in #891 (PRESERVE THIS)
He bumped chat text up from `text-sm` (0.875rem=14px) to **15px** because sm felt
too small. Conversions made:
- MessageRow author name `<span>` & `<h3>`: `text-sm` → `text-[15px]`
- markdown body / mentionChip: `text-sm` → `text-[15px]`
- `globals.css` `.mention-highlight`: added `font-size: 15px`, radius 0.375rem→4px
- also touched MessageTimeline, SystemMessageRow, MessageThreadSummaryRow

## The crux
Tailwind v4 here uses STOCK text tokens (no `@theme` override). Stock scale:
text-sm=14px, text-base=16px. **15px sits between them — no stock token exists.**
That's WHY Kenny reached for arbitrary px.

## tho's directive
- Use rem + Tailwind tokens wherever possible.
- Preserve Kenny's visual intent (the 15px chat sizing).
- Outcome MAY be to define/update a text-size token to yield 15px in rem — but
  only if stock tokens genuinely can't deliver the look. Take a critical pass:
  prefer a stock token if it looks right; introduce a custom token only if needed.

## Scope: timeline + thread render path only
MessageRow, markdown.tsx, mentionChip.ts, SystemMessageRow,
MessageThreadSummaryRow, MessageTimeline, and the relevant globals.css rules
(incl. `.mention-highlight` px font-size). Verify zoom works after.

## Codify (fix #2)
No custom lint exists. Add a guard (Biome rule or CI grep) flagging new
`text-[NNpx]` / px `fontSize` in the desktop app + a note in AGENTS.md/CLAUDE.md.
