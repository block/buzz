import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

/// How far a finger must travel downward before the keyboard is dismissed.
///
/// Large enough that reading-speed scrolling never trips it, small enough that
/// a deliberate "push it away" swipe feels immediate.
const keyboardDismissDragThreshold = 48.0;

/// Dismiss the software keyboard when the user deliberately drags downward.
///
/// Flutter only offers `ScrollViewKeyboardDismissBehavior.manual` and `.onDrag`
/// — and `.onDrag` fires the instant a drag begins, so a one-pixel scroll to
/// re-read a message would tear the keyboard away. Neither is available on
/// `ScrollablePositionedList` regardless, which is what both message lists use.
///
/// So: accumulate raw finger travel and only unfocus once a downward drag
/// clears [keyboardDismissDragThreshold]. Any upward movement resets the
/// accumulator, so a scroll that wanders up and down never adds up to a
/// dismissal.
///
/// This tracks the *finger*, not the scroll direction, so it behaves the same
/// in the reversed channel timeline and the top-anchored thread list. It is not
/// interactive dismissal — the keyboard animates away on the OS's own curve
/// rather than following the finger, which Flutter can't do without a native
/// plugin (iOS `UIScrollView.keyboardDismissMode`, Android
/// `WindowInsetsAnimationController`).
class KeyboardDismissOnDrag extends HookWidget {
  final Widget child;
  final VoidCallback? onUserScrollStart;

  const KeyboardDismissOnDrag({
    super.key,
    this.onUserScrollStart,
    required this.child,
  });

  @override
  Widget build(BuildContext context) {
    // A ref, not state: accumulating travel must never trigger a rebuild of
    // the message list this wraps.
    final downwardTravel = useRef(0.0);

    bool handle(ScrollNotification notification) {
      if (notification is ScrollStartNotification) {
        if (notification.dragDetails != null) onUserScrollStart?.call();
        downwardTravel.value = 0;
        return false;
      }
      if (notification is ScrollEndNotification) {
        downwardTravel.value = 0;
        return false;
      }

      // Overscroll counts as much as ordinary scrolling. You focus the composer
      // at the resting end of the list far more often than not, and there a
      // downward drag moves the position nowhere — it is pure overscroll, and
      // reports `OverscrollNotification` instead. Listening only for
      // `ScrollUpdateNotification` meant the most common dismissal gesture in
      // both lists produced no notification this saw at all.
      final DragUpdateDetails? drag;
      if (notification is ScrollUpdateNotification) {
        drag = notification.dragDetails;
      } else if (notification is OverscrollNotification) {
        drag = notification.dragDetails;
      } else {
        return false;
      }

      // Null during a momentum fling — only a finger still on the glass should
      // dismiss, so a fling that coasts downward leaves the keyboard up.
      if (drag == null) {
        downwardTravel.value = 0;
        return false;
      }

      final dy = drag.delta.dy;
      if (dy <= 0) {
        downwardTravel.value = 0;
        return false;
      }

      downwardTravel.value += dy;
      if (downwardTravel.value < keyboardDismissDragThreshold) return false;
      downwardTravel.value = 0;
      dismissKeyboard(context);
      return false;
    }

    return NotificationListener<ScrollNotification>(
      onNotification: handle,
      child: child,
    );
  }
}

/// Drop focus so the OS retracts the keyboard. No-op when nothing is focused
/// or the keyboard is already down, so callers can fire it freely.
void dismissKeyboard(BuildContext context) {
  // Read the view's own insets, not MediaQuery's. `Scaffold` strips the bottom
  // view inset from its body while `resizeToAvoidBottomInset` is on — it has
  // already resized to account for it — so a MediaQuery read from anywhere
  // inside the body reports 0 whether the keyboard is up or not, which made
  // this gate a permanent no-op for both message lists and the compose bar.
  if (View.of(context).viewInsets.bottom <= 0) return;
  FocusManager.instance.primaryFocus?.unfocus();
}
