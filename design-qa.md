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

---

# Living Ship alignment design QA

## Evidence

- Reference: `/var/folders/nc/7ggx382n68z9pfms6sg20r7h0000gn/T/TemporaryItems/NSIRD_screencaptureui_vk6Fyc/Screenshot 2026-08-02 at 1.58.01 pm.png`
- Implementation: `desktop/test-results/living-ship/01-overview.png`
- Comparison: `desktop/test-results/living-ship/alignment-comparison-full.png`
- Focused comparisons: `desktop/test-results/living-ship/alignment-comparison-aft.png` and `desktop/test-results/living-ship/alignment-comparison-forward.png`
- Logical viewport: 1510 x 874 CSS pixels in both captures
- Capture density: reference 3020 x 1748 at 2x; implementation 1510 x 874 at 1x, scaled with nearest-neighbour sampling for the side-by-side comparison
- State note: the reference uses the installed advisers with Ship's Office selected; the implementation uses deterministic E2E advisers with no selected compartment. The underlying ship artwork, canvas geometry and viewport are equivalent for alignment review.

## Findings

### Geometry and alignment

- Resolved P1: room controls and agent sprites were projected through a legacy 1920 x 981 grid while the final artwork is 1754 x 896 pixels. This placed every overlay too far left and too high.
- The implementation now derives room bounds and agent anchors from the native 1754 x 896 artwork coordinate system.
- Aft: DSE Operator Room and Plans Room controls now sit inside their illuminated compartment outlines; their advisers share the same anchors.
- Forward: CIC and Chart House align to the upper row, Wardroom and Meeting Room to the middle row, and Ship's Office and Supply Office to the lower row.
- No remaining P0, P1 or P2 alignment defects found at the reported window size.

### Visual fidelity

- Ship artwork, palette, lighting, typography, borders and copy are unchanged.
- The native image aspect ratio is preserved, so overlays remain locked to the artwork as the window scales.
- P3 follow-up only: rooms with several collaborating advisers are intentionally busy and their short labels can cluster. This does not affect compartment hit targets or the reported alignment defect.

## Verification

- Coordinate regression test covers the CIC room and Operations adviser as representative native-art anchors.
- Living Ship Playwright journey passes at the reported 1510 x 874 logical viewport and captures the corrected full screen.
- Final result: passed.
