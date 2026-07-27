# Preset harness logos — provenance

Third-party marks bundled to identify tier-2 preset harnesses in the runtime
gallery (`PRESET_LOGOS` in `desktop/src/features/onboarding/ui/RuntimeIcon.tsx`).
Nominative use only — each mark identifies its own vendor's harness.

Add a row here when adding a preset logo; only bundle marks whose upstream
license permits redistribution.

| File | Upstream | Commit | License | Source path | Modifications |
|---|---|---|---|---|---|
| `hermes.png` | [NousResearch/hermes-agent](https://github.com/NousResearch/hermes-agent) | `6ad632b` | MIT © 2025 Nous Research | `website/static/img/logo.png` | Cropped the baked-in border frame, padded to square, resized to 64×64, quantised to a 16-colour palette |
| `openclaw.svg` | [openclaw/openclaw](https://github.com/openclaw/openclaw) | `b06f40a` | MIT © 2026 OpenClaw Foundation | `ui/public/favicon.svg` | Removed the SMIL animation elements (renders the upstream rest pose statically — verified pixel-identical to the upstream frame at t=0); minified paths |
| `omp.svg` | [can1357/oh-my-pi](https://github.com/can1357/oh-my-pi) | `667111575ebba136dadfd6989379e7f67e0d40d9` | MIT © 2025 Mario Zechner; © 2025–2026 Can Bölük | `assets/icon.svg` | None |
| `kimi.png` | [MoonshotAI/kimi-cli](https://github.com/MoonshotAI/kimi-cli) | `4a550effdfcb29a25a5d325bf935296cc50cd417` | Apache-2.0; NOTICE: Kimi Code CLI © 2025 Moonshot AI | `web/public/logo.png` | None |
| `grok.svg` | [SpaceXAI brand guidelines](https://x.ai/legal/brand-guidelines) | Retrieved 2026-07-25 | xAI Brand Guidelines: marks may be used to accurately refer to xAI or its services; logos must be used exactly as provided | `SpaceXAI_Grok_Assets.zip` → `Grok_Logomark_Dark.svg` | None |

`amp.png` and `opencode.svg` predate this file; their provenance was not
recorded when they were added. Cursor intentionally uses the generic terminal
fallback: Cursor's official brand page offers downloadable assets, but neither
that page nor its Terms of Service grants third parties permission to
redistribute them. The previous unproven `cursor.png` was removed.
