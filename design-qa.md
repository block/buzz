# Command Adviser Naval UI Design QA

Reference:
`/Users/matthewwarren/.codex/generated_images/019f91f9-3cb6-7323-bc4f-8bcfcc7e0373/call_Tl4utET31OeAI23lycQaKv22.png`

Validated captures:

- `desktop/test-results/command-adviser-naval-ui/default-briefing.png`
- `desktop/test-results/command-adviser-naval-ui/expanded-evidence.png`

Viewport: 1280 x 720.

## Findings

- P0: none.
- P1: resolved. The first rendered iteration used the host light surface and
  placed generation metadata ahead of the command content. The console now owns
  a deep-navy surface, the completed-run metadata is in the evidence disclosure,
  and the first decision is visible in the initial viewport.
- P2: resolved. The hero and model controls were too tall, and the adviser team
  interrupted the command reading order. The hero is now shallow, routing is a
  compact row, and the brief precedes the symbolic team.
- P3: the surrounding shared workspace rail follows the user's selected app
  theme rather than forcing Command Adviser navy across unrelated Buzz
  workspace routes. This is intentionally retained to keep the refresh scoped.

The final capture retains the selected direction's navy and brass palette,
local HMAS Supply image and badge, compact model routing, decision-first
hierarchy, symbolic adviser identity, and collapsed evidence boundary without
adding decorative instrument-panel effects.

final result: passed
