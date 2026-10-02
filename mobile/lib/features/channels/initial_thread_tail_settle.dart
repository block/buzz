import 'package:flutter/widgets.dart';
import 'package:scrollable_positioned_list/scrollable_positioned_list.dart';

/// Settles an ordinary thread open on the latest hydrated reply after layout.
///
/// Scheduling again before completion invalidates callbacks aimed at an older
/// tail, allowing a rebuild with newly arrived replies to choose the target.
class InitialThreadTailSettle {
  var _generation = 0;
  var _isComplete = false;

  /// Whether no more settling is needed.
  ///
  /// Completion occurs when there is no tail target, the target is already
  /// visible after hydration has settled, or the scheduled scroll finishes.
  bool get isComplete => _isComplete;

  /// Permanently abandons initial settling and invalidates queued callbacks.
  ///
  /// This is terminal: later scheduling remains disabled even if the user
  /// returns to the tail and resumes ordinary follow behavior.
  void abandon() {
    _generation++;
    _isComplete = true;
  }

  /// Schedules a settle after each hydrated thread layout until [isComplete].
  ///
  /// A later schedule replaces an earlier target while replies are still
  /// arriving. The final target is left in place when already visible; otherwise
  /// it scrolls into the viewport between the measured top and bottom overlays.
  void schedule({
    required BuildContext context,
    required ItemScrollController controller,
    required ItemPositionsListener positionsListener,
    required int? targetIndex,
    required double hiddenTopFraction,
    required double hiddenBottomFraction,
    required VoidCallback onSettled,
  }) {
    if (_isComplete) return;

    final generation = ++_generation;
    if (targetIndex == null) {
      _isComplete = true;
      onSettled();
      return;
    }

    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!context.mounted || generation != _generation) return;

      // Let events received during hydration rebuild the list before committing
      // the target. That rebuild schedules a new generation at the current tail.
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!context.mounted ||
            !controller.isAttached ||
            generation != _generation) {
          return;
        }
        final targetIsFullyVisible = positionsListener.itemPositions.value.any(
          (position) =>
              position.index == targetIndex &&
              position.itemLeadingEdge >= hiddenTopFraction &&
              position.itemTrailingEdge <= 1 - hiddenBottomFraction,
        );
        // Short threads already expose their tail from the top anchor. Moving
        // that fully visible target down would only add empty space above the
        // head. A clipped tail still takes the measured correction path.
        if (targetIsFullyVisible) {
          _isComplete = true;
          onSettled();
          return;
        }
        // This package uses a temporary second list for distant targets. The
        // caller keeps the hydrated viewport unpainted until this one-frame
        // placement completes, so that implementation detail cannot appear as
        // an entry bounce.
        controller
            .scrollTo(
              index: targetIndex,
              alignment: hiddenTopFraction,
              duration: const Duration(milliseconds: 1),
            )
            .whenComplete(() {
              if (generation != _generation) return;
              _isComplete = true;
              onSettled();
            });
      });
      // A post-frame callback does not itself request the frame in which it
      // runs. Slow hydration can otherwise leave this settle parked until an
      // unrelated redraw.
      WidgetsBinding.instance.scheduleFrame();
    });
  }
}
