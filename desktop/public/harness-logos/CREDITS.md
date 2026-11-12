# Preset harness logos — provenance

Third-party marks bundled to identify tier-2 preset harnesses in the runtime
gallery (`PRESET_LOGOS` in `desktop/src/features/onboarding/ui/RuntimeIcon.tsx`).
Nominative use only — each mark identifies its own vendor's harness.

Add a row here when adding a preset logo; only bundle marks whose upstream
license permits redistribution.

| File | Upstream | Commit | License | Source path | Modifications |
|---|---|---|---|---|---|
| `devin.svg` | [Cognition Devin documentation](https://docs.devin.ai/cli) | Retrieved 2026-07-27 | Cognition trademark; nominative use to identify the Devin harness | Official documentation `logo/favicon.svg` | Added the official black mark to a white square canvas so it remains legible in both app themes |
| `hermes.png` | [NousResearch/hermes-agent](https://github.com/NousResearch/hermes-agent) | `6ad632b` | MIT © 2025 Nous Research | `website/static/img/logo.png` | Cropped the baked-in border frame, padded to square, resized to 64×64, quantised to a 16-colour palette |
| `openclaw.svg` | [openclaw/openclaw](https://github.com/openclaw/openclaw) | `b06f40a` | MIT © 2026 OpenClaw Foundation | `ui/public/favicon.svg` | Removed the SMIL animation elements (renders the upstream rest pose statically — verified pixel-identical to the upstream frame at t=0); minified paths |
| `omp.svg` | [can1357/oh-my-pi](https://github.com/can1357/oh-my-pi) | `667111575ebba136dadfd6989379e7f67e0d40d9` | MIT © 2025 Mario Zechner; © 2025–2026 Can Bölük | `assets/icon.svg` | None |
| `pi.svg` | [earendil-works/pi-website](https://github.com/earendil-works/pi-website) | `2f5e410b97474d0a34ec2500aa1aa58d6c3f992c` | MIT © 2026 Earendil Inc. and contributors | `src/favicon.svg` | None |
| `kimi.png` | [MoonshotAI/kimi-cli](https://github.com/MoonshotAI/kimi-cli) | `4a550effdfcb29a25a5d325bf935296cc50cd417` | Apache-2.0; NOTICE: Kimi Code CLI © 2025 Moonshot AI | `web/public/logo.png` | None |
| `grok.svg` | [SpaceXAI brand guidelines](https://x.ai/legal/brand-guidelines) | Retrieved 2026-07-25 | xAI Brand Guidelines: marks may be used to accurately refer to xAI or its services; logos must be used exactly as provided | `SpaceXAI_Grok_Assets.zip` → `Grok_Logomark_Dark.svg` | None |

## Inline SVG marks (`RUNTIME_MARKS`)

Monochrome marks inlined as `currentColor` paths in
`desktop/src/features/onboarding/ui/HarnessMarks.tsx` (no files under
`public/`), so they adapt to dark/light themes without bitmap filters.

| Mark | Upstream | Version/Commit | License | Source path | Modifications |
|---|---|---|---|---|---|
| Goose | [block/goose](https://github.com/block/goose) | `305849b71709b95b86ed9f11bd3bc939899c0aab` | Apache-2.0 © Block, Inc. | `documentation/static/img/goose.svg` | `fill="#101010"` → `currentColor`; dropped the redundant clipPath wrapper |
| Cursor | [simple-icons](https://github.com/simple-icons/simple-icons) | `16.27.1` (slug `cursor`) | CC0-1.0 (path data); nominative use of the Cursor mark to identify Cursor's harness | `icons/cursor.svg` | `fill` → `currentColor` |

Codex deliberately has **no** bundled mark: the OpenAI blossom was removed
from simple-icons in v16 at the vendor's request, so we do not ship it —
Codex renders `RuntimeIcon`'s neutral terminal-glyph fallback instead.

`amp.png` and `opencode.svg` predate this file; their provenance was not
recorded when they were added. Cursor previously used the generic terminal
fallback because Cursor's own brand page does not grant redistribution; the
CC0-licensed simple-icons path (above) resolves that, mirroring the grok
nominative-use precedent. The previous unproven `cursor.png` was removed, as
were the unproven `chatgpt.png` and `goose.png` builtin-runtime bitmaps
(replaced by the inline marks above).
