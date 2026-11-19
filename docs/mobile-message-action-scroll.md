# Message-action body scroll handoff

Only `showMessageActions` opts into `ModalScrollHandoff`. Image actions and other
sheets keep their existing scrolling. The helper retains the modal route,
shared chrome, stock handle drag, barrier dismissal, and accessibility action.

The explicit, non-primary, downward vertical scroll position allocates a user
update between content and the existing route animation. Crossing the content's
leading edge spends that update's remainder on sheet translation; reversing
spends the remainder on content after fully reopening. Nested scroll positions
never feed this allocator. Programmatic and ballistic scrolling cannot start a
sheet drag. There is no extra gesture recognizer or replacement route.

The enclosing `BottomSheet` supplies its animation, full rendered height, drag
start/end callbacks, and cancellation close callback. Entrance must finish
before handoff can begin. Geometry is read again for every update. The route
owns animation disposal; the helper owns only its controller and gesture token.

Release policy follows Flutter 3.47.9's `material/bottom_sheet.dart`: downward
velocity strictly above 700 logical px/s closes; otherwise progress strictly
below 0.5 closes. A raw upward velocity strictly below -700 vetoes the distance
rule. The wrapper captures the raw terminal event before Flutter negates it or
adds carried scroll momentum. Cancellations and activity interruptions reopen,
including when another active route covers the sheet. Departing routes are not
reopened. Reduced motion preserves direct manipulation but settles immediately.
The route close callback returns null, leaving message selection and composer
focus restoration with their existing owners.

## Automated validation

From the repository root:

```sh
. ./bin/activate-hermit
just mobile-install mobile-check mobile-test
```

Focused journeys run with:

```sh
. ./bin/activate-hermit
cd mobile
flutter test test/features/channels/message_actions_test.dart \
  test/shared/widgets/modal_scroll_handoff_test.dart
```

The real message-action journey uses a 375×667 viewport, 2× text, management
permissions, and reminder/thread actions. It proves overflow, touch reachability,
same-update geometry, reversal, horizontal isolation, settle, action taps,
reactions/picker navigation, draft preservation, and cancellation-only focus
restoration. Shared journeys cover exact velocity/progress boundaries,
interruptions, bouncing offsets, carried momentum, layout changes, short bodies,
reduced motion, nested scrolling, and unchanged handle/accessibility dismissal.
Replacing the message-action opt-in with an ordinary `SingleChildScrollView`
was verified to fail the pre-release geometry assertion by 40 logical pixels.

Final validation on 20 November 2026: `just mobile-check` passed formatting and
analysis with no issues; `just mobile-test` passed all nine gateway recipe
checks, 5,140 Flutter tests (71 skipped), and 27 crypto tests. The focused files
passed 61 tests. `just file-size-check` and `git diff --check` passed. No
preexisting test failures were observed in the final run. Independent agent
review and follow-up cleared the covered-route interruption regression.

## Native validation boundary (19 November 2026)

Before editing, the environment was inspected again: Xcode and AXe were
installed; an iPhone 17 Pro simulator running iOS 27.2 was booted. No Buzz app
appeared in `xcrun simctl listapps booted`, and this worktree had no package
configuration or simulator app build. Extension discovery exposed **no iOS
Simulator MCP**. The simulator feedback-loop skill requires that MCP and stops
native interaction when it is absent. Accordingly no AXe-only substitute was
used, no native build/installation was attempted, and no connected community,
overflowing native menu, gestures, screenshots, or recordings were observed.
Widget platform variants are not native confirmation.

When that tooling is available, run the unchanged baseline or its parent commit
first, then this change, using `just mobile-dev` and an accessible community.
With a nonempty focused draft, long-press an owned message and increase text size
until lower actions are initially offscreen. Compare body and handle drags;
scroll down the actions, cross the top without lifting, reverse, and test short,
long/slow, fast-down, strong-up, cancellation, barrier/back, and thread messages.
Confirm lower actions remain reachable, the whole sheet follows the finger,
settle-open does not restore composer focus, cancellation does, and the draft
survives. Test the short system-message reaction menu too. Separate human
confirmation remains outstanding.
