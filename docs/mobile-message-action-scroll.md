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

Release policy follows Flutter 3.47.9's `material/bottom_sheet.dart`, with one
deliberate difference. As in the stock sheet, a release with the sheet displaced
closes when downward velocity is strictly above 700 logical px/s, and otherwise
when progress is strictly below 0.5 of the sheet height. The difference: a raw
upward velocity strictly below -700 vetoes the progress rule.

Where the gesture began does not matter. A swipe that starts with content
scrolled away from the top, by a little or a lot, or that scrolls content down
and back in the same touch, closes on velocity like one that began at the top,
provided the finger has carried content to the top and displaced the sheet by
the time it lifts. The guards against accidental dismissal are these:

- A release with the sheet not displaced by the finger never closes, however
  fast. A flick that leaves content short of the top only scrolls.
- Ballistic and programmatic scrolling cannot start a sheet drag, so a content
  fling that reaches the top after the finger lifted stops there.
- Scroll momentum carried into a new touch is not added to the release
  velocity.
- A slow return across the top that displaces the sheet less than half way
  settles open.

A parity test releases the same pointer gestures through the stock sheet drag
and through the body, so a Flutter upgrade that moves either stock threshold
fails it. The wrapper captures the raw terminal event before Flutter negates it or
adds carried scroll momentum. Cancellations and activity interruptions reopen,
including when another active route covers the sheet. Flutter delivers a
system-cancelled pointer to an accepted drag as an ordinary end, so the body
listens for the pointer cancellation itself and treats that end as an
interruption: a deep pull that the system cancels reopens. Departing routes are not
reopened. Reduced motion preserves direct manipulation but settles immediately.
The route close callback returns null, leaving message selection and composer
focus restoration with their existing owners.

The sheet has one drag owner at a time. A body gesture owns the sheet from its
first sheet movement until it settles it. If anything else moves the route
animation meanwhile, such as a second finger dragging the handle, the body
gesture gives the sheet up for the rest of that touch: it neither moves nor
settles the sheet, and only scrolls content. The handle's own release then
settles the sheet. A body gesture cannot begin a sheet drag while the sheet is
already displaced, and a body touch that lands during a handle drag joins the
stock sheet recognizer rather than the scroll position.

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
shared tests cover the release boundaries, releases after scrolled content
(new touch and same touch), stock parity, interruptions, bouncing offsets,
carried momentum, a real pointer cancellation, a second finger on the handle,
layout changes, short bodies, reduced motion, and nested
scrolling.

## Manual native checklist

Widget tests are not native confirmation. Run `just mobile-dev` against an
accessible community, with a nonempty focused draft, and long-press an owned
message. Increase text size until the lower actions start offscreen.

1. Scroll to the last action and back; every action is reachable.
2. From the top, drag the body down slowly past half the sheet and release: the
   menu closes. Release before half: it settles open.
3. From the top, flick the body down: the menu closes.
4. Scroll the list up slightly and lift, then swipe the body down hard so the
   sheet moves: the menu closes. Repeat after scrolling to the last action.
5. From the top, in one touch, drag up to scroll the list slightly, then swipe
   down hard without lifting so the sheet moves: the menu closes.
6. Scroll down and lift, then flick so the list flings to the top after the
   finger has lifted: the sheet does not move and the menu stays open. Drag
   slowly back across the top so the sheet moves less than half way and
   release: it settles open.
7. Cross the top without lifting, then reverse: the sheet reopens fully before
   content scrolls, and it follows the finger throughout.
8. Pull the sheet down, then release with a strong upward flick: it stays open.
9. Compare the same drags on the handle; check barrier tap and system back.
10. Settling open leaves composer focus alone; dismissing restores it. The
    draft survives both.
11. Pull the body down slightly and hold. With a second finger drag the handle
    past half way, then lift the body finger: the menu stays under the handle
    finger. Lift that finger above half way: the menu settles open.
12. Repeat on a thread message and on the short system-message reaction menu.
