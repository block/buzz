import 'package:flutter/widgets.dart';
import 'package:scrollable_positioned_list/scrollable_positioned_list.dart';

/// Settles an ordinary thread open on the latest hydrated reply after layout.
///
/// Scheduling again before completion invalidates callbacks aimed at an older
/// tail, allowing a rebuild with newly arrived replies to choose the target.
class InitialThreadTailSettle {
  var _generation = 0;
  var _isComplete = false;

  bool get isComplete => _isComplete;

  void schedule({
    required BuildContext context,
    required ItemScrollController controller,
    required ItemPositionsListener positionsListener,
    required int? targetIndex,
    required double hiddenBottomFraction,
  }) {
    if (_isComplete) return;

    final generation = ++_generation;
    if (targetIndex == null) {
      _isComplete = true;
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
              position.itemLeadingEdge >= 0 &&
              position.itemTrailingEdge <= 1 - hiddenBottomFraction,
        );
        // Short threads already expose their tail from the top anchor. Moving
        // that fully visible target down would only add empty space above the
        // head. A clipped tail still takes the measured correction path.
        if (targetIsFullyVisible) {
          _isComplete = true;
          return;
        }
        controller
            .scrollTo(
              index: targetIndex,
              alignment: 0.0,
              duration: const Duration(milliseconds: 1),
            )
            .whenComplete(() {
              if (generation == _generation) _isComplete = true;
            });
      });
    });
  }
}
