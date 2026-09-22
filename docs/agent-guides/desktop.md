## Desktop App

The desktop app is Tauri 2 + React 19 + Vite + Tailwind CSS. Features are
organized under `desktop/src/features/`. Biome handles linting and formatting.

```bash
just desktop-dev   # web-only dev server (faster iteration)
just dev           # full Tauri app with native shell
```

### Text sizing & zoom (use rem, never px)

The desktop app implements Cmd +/- zoom by scaling the root `<html>`
font-size (`desktop/src/app/useWebviewZoomShortcuts.ts`) and pinning the native
webview zoom. **Only rem-based text scales with zoom — hardcoded px text sizes
are frozen.**

So for any readable text, reach for rem-based Tailwind tokens, never arbitrary
px:

- ✅ Stock rem tokens (`text-base`, `text-sm`, `text-xs`, …) for general
  interface text. All of these derive from the virtual typography rem and
  therefore follow the user's font-size preference and Cmd +/- zoom.
- ✅ Conversation text uses the named `text-message` token. Its
  **Smaller / Default / Larger contract is 13 / 14 / 15px** before keyboard
  zoom. Author names use the same conversation-size step; timestamps, system
  rows, code, and reactions are deliberate neighboring steps on the shared
  virtual-rem ramp. Keep those relationships tokenized rather than restoring a
  fixed 16px chat baseline or hardcoding preference-specific values in
  components.
- ✅ The `text-2xs` (0.6875rem / 11px at a 16px virtual rem) and `text-3xs`
  (0.5rem / 8px at a 16px virtual rem) meta-text
  tokens (in `desktop/tailwind.config.js` under `theme.extend.fontSize`) for the
  sub-`text-xs` ramp — timestamps, count badges, tracking labels, tiny glyphs.
  These replaced the dozens of arbitrary `text-[…rem]` literals that had drifted
  apart pixel-by-pixel; keep meta text on these two tokens, not new arbitrary
  values.
- ❌ `text-[15px]`, `text-[13px]`, CSS `font-size: 15px` — px froze against zoom
  and caused the message-timeline regression (PR #891).
- ❌ Arbitrary rem literals too: `text-[0.6875rem]`, `text-[0.9rem]`, etc. They
  zoom fine but re-fragment the scale we consolidated. Use a named token.

Prefer stock tokens — they're rem and zoom-safe. Only if a design genuinely
needs a size the stock/`2xs`/`3xs` scale can't express should you **add a
rem-based token** (in `desktop/tailwind.config.js` under `theme.extend.fontSize`)
rather than an arbitrary literal. A CI guard (`pnpm check:px-text`, in
`desktop/scripts/check-px-text.mjs`) scans all of `desktop/src` and fails on any
new arbitrary text-size literal — px **or** rem/em. Genuinely decorative glyphs
(e.g. the `text-[6rem]` avatar emoji) are allowlisted by `path:line` in that
script.

### Community Switching

The desktop app supports multiple communities (each backed by a different relay).
Switching communities does **not** reload the page — it uses React key-based
remounting. `<AppReady key={communityKey} />` in `App.tsx` forces the entire
community-scoped subtree to unmount and remount with fresh state.

**Module-level singletons must be explicitly reset.** React remounting only
clears React state (useState, useRef, context). Module-level variables (Maps,
class instances, cached promises) survive across remounts. Every community-scoped
singleton needs a reset function wired into `resetCommunityState()` in
`desktop/src/features/communities/useCommunityInit.ts`.

`resetCommunityState()` is the canonical inventory of community-scoped
singletons. **If you add a new module-level cache, Map, or class instance that
holds community-scoped data, add its reset there in the same change.** Failure
to do so causes data from the old community to leak into the new one. Avoid
duplicating its complete reset list here; the implementation is the source of
truth.

Key files:
- `desktop/src/app/App.tsx` — community key, init gate, remount boundary
- `desktop/src/features/communities/useCommunityInit.ts` — `resetCommunityState()`, applies config to Tauri backend
- `desktop/src/main.tsx` — provider hierarchy (`QueryClientProvider` > `App`)

---

### Mention editor contract

Autocomplete inserts a literal full label and a separator, including multi-word
names. Only autocomplete settlement may move the caret past that separator;
internal label spaces and deliberate ArrowLeft/click movement must be respected.
See `docs/mention-editor.md` and `desktop/tests/e2e/mention-spacing.spec.ts`.

Selected mention labels bind exact keys, including same-name teammates and
persistent automatic addresses. Use the returned label from registration for
insert/restore/remove. Ambiguous manually typed names must fail visibly without
clearing the draft in chat, edit, and standalone forum consumers; never fan out
silently to all identities sharing a name. See `docs/mention-editor.md`.
