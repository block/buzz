import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../theme/theme.dart';
import 'conversation_layout.dart';

/// The width the user dragged the tablet thread pane to, or null for the
/// default. Kept for the app session, across threads and rotations.
final conversationThreadWidthProvider =
    NotifierProvider<ConversationThreadWidth, double?>(
      ConversationThreadWidth.new,
    );

/// Holds the dragged thread pane width; null restores the default.
class ConversationThreadWidth extends Notifier<double?> {
  @override
  double? build() => null;

  // A setter would read as field assignment at call sites; keep it a method.
  // ignore: use_setters_to_change_properties
  void set(double? width) => state = width;
}

/// Step a screen reader's increase and decrease actions resize the pane by.
const double conversationThreadResizeStep = 40;

/// Touch width of the handle between a conversation and its thread pane.
const double conversationThreadHandleWidth = Grid.xs;

/// The draggable divider beside a resizable tablet pane: between a
/// conversation and the thread beside it, or between the sidebar and the
/// conversation.
///
/// Dragging moves the divider and resizes the pane within
/// [minWidth]..[maxWidth]. Screen readers adjust it as a slider.
class ConversationThreadDivider extends HookWidget {
  const ConversationThreadDivider({
    required this.width,
    required this.maxWidth,
    required this.onDragStart,
    required this.onResize,
    required this.onDragEnd,
    this.label = 'Thread width',
    this.minWidth = conversationThreadPaneMinWidth,
    this.leading = false,
    super.key,
  });

  /// The thread pane's current width.
  final double width;

  /// The widest the pane may be dragged to right now.
  final double maxWidth;

  /// Called as a drag begins, before the first [onResize].
  final VoidCallback onDragStart;

  /// Called with each new width, already clamped.
  final ValueChanged<double> onResize;

  /// Called when a drag ends or is cancelled.
  final VoidCallback onDragEnd;

  /// The narrowest the pane may be dragged to.
  final double minWidth;

  /// The slider's screen-reader label.
  final String label;

  /// Whether the pane sits before the divider, so dragging toward the end
  /// widens it. A thread pane sits after it, at the window's end.
  final bool leading;

  double _clamp(double value) =>
      value.clamp(minWidth, maxWidth < minWidth ? minWidth : maxWidth);

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    // Measured from where the drag began, so the divider keeps tracking the
    // finger after it pushes past a limit and comes back.
    final dragStart = useRef<({double width, double dx})?>(null);
    String describe(double value) => '${value.round()} points';
    final larger = _clamp(width + conversationThreadResizeStep);
    final smaller = _clamp(width - conversationThreadResizeStep);
    return Semantics(
      slider: true,
      label: label,
      value: describe(width),
      increasedValue: describe(larger),
      decreasedValue: describe(smaller),
      onIncrease: larger == width ? null : () => onResize(larger),
      onDecrease: smaller == width ? null : () => onResize(smaller),
      child: MouseRegion(
        cursor: SystemMouseCursors.resizeColumn,
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          onHorizontalDragStart: (details) {
            dragStart.value = (width: width, dx: details.globalPosition.dx);
            onDragStart();
          },
          onHorizontalDragUpdate: (details) {
            final start = dragStart.value;
            if (start == null) return;
            final moved = details.globalPosition.dx - start.dx;
            onResize(_clamp(start.width + (leading ? moved : -moved)));
          },
          onHorizontalDragEnd: (_) {
            dragStart.value = null;
            onDragEnd();
          },
          onHorizontalDragCancel: () {
            dragStart.value = null;
            onDragEnd();
          },
          child: Center(
            child: Stack(
              alignment: Alignment.center,
              children: [
                SizedBox(
                  width: 1,
                  height: double.infinity,
                  child: ColoredBox(color: colors.outlineVariant),
                ),
                // A grabber makes the divider discoverable as a handle.
                DecoratedBox(
                  decoration: BoxDecoration(
                    color: colors.outline,
                    borderRadius: BorderRadius.circular(Radii.full),
                  ),
                  child: const SizedBox(width: 5, height: 36),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
