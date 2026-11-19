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

Release policy follows Flutter 3.47.9's `material/bottom_sheet.dart`, with two
deliberate differences. As in the stock sheet, downward velocity strictly above
700 logical px/s closes, and otherwise progress strictly below 0.5 of the sheet
height closes. The differences:

- The velocity rule applies only to a gesture that began with content at its
  leading edge. A gesture that began mid-content closes on progress alone, so a
  fast flick back to the top that carries the sheet a little way settles open
  with content at the top instead of dismissing the menu.
- A raw upward velocity strictly below -700 vetoes the progress rule.

A parity test releases the same pointer gestures through the stock sheet drag
and through the body, so a Flutter upgrade that moves either stock threshold
fails it. The wrapper captures the raw terminal event before Flutter negates it or
adds carried scroll momentum. Cancellations and activity interruptions reopen,
including when another active route covers the sheet. Departing routes are not
reopened. Reduced motion preserves direct manipulation but settles immediately.
The route close callback returns null, leaving message selection and composer
focus restoration with their existing owners.

Content shorter than its viewport still uses always-scrollable physics on
purpose: a downward body drag on a short menu, such as the system-message
reaction menu, moves the sheet exactly as a drag on the handle does.

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

The message-action journey uses a 375×667 viewport, 2× text, management
permissions, and reminder/thread actions on iOS and Android settings. The
shared tests cover the release boundaries, stock parity, interruptions,
bouncing offsets, carried momentum, layout changes, short bodies, reduced
motion, and nested scrolling.

## Manual native checklist

Widget tests are not native confirmation. Run `just mobile-dev` against an
accessible community, with a nonempty focused draft, and long-press an owned
message. Increase text size until the lower actions start offscreen.

1. Scroll to the last action and back; every action is reachable.
2. From the top, drag the body down slowly past half the sheet and release: the
   menu closes. Release before half: it settles open.
3. From the top, flick the body down: the menu closes.
4. Scroll down, then flick back up the list so the sheet moves slightly: the
   menu stays open, settled at the top.
5. Cross the top without lifting, then reverse: the sheet reopens fully before
   content scrolls, and it follows the finger throughout.
6. Pull the sheet down, then release with a strong upward flick: it stays open.
7. Compare the same drags on the handle; check barrier tap and system back.
8. Settling open leaves composer focus alone; dismissing restores it. The draft
   survives both.
9. Repeat on a thread message and on the short system-message reaction menu.
